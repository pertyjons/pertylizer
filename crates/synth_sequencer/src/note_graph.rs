//! Note Grid — modular note-processing graphs (linear-first).
//!
//! A [`NoteGraph`] is a project-level asset (pooled in `Song`, referenced by
//! id) that transforms a stream of expanded notes. In v1 the `NoteStream` spine
//! is constrained **linear** (one chain, no splitters/merge); the data model is
//! already graph-shaped so the DAG escalation only relaxes that validation.
//!
//! The pure per-tick evaluation core: host-key seed decorrelation, nodes
//! wrapping the existing `NoteProcessor` DSP, plus native modules (a Euclidean
//! generator; `Value`/`Gate` modulators land in a later phase). The pool lives
//! in `Song` and patterns bind a graph by id; the engine expands a bound
//! pattern through its graph.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use synth_core::script::{BoundScript, NoteField, NoteOutputs, RegisterFile, ScriptInput};

use crate::ids::{NoteGraphId, NoteId, NoteModuleId, PatternId, TrackId};
use crate::note::Note;
use crate::note_processor::{
    ExpandedNote, ExpansionBuffer, MAX_DELAY_REPEATS, MAX_ENV_STREAM_WINDOW, MAX_NOTE_DELAY_TICKS,
    MAX_RATCHET_SUBDIVISIONS, NoteProcessor, lookback_pool, seed_source_at_tick,
};
use crate::pitch::{Pitch, Velocity};
use crate::time::{Duration, PatternTick};
use crate::track::TrackColor;
use synth_core::Bpm;
use synth_core::NormalizedValue;
use synth_core::hash::splitmix64_unit;

/// Stable per-host seed salt (plan §1.2).
///
/// The referencing host's id, folded to a `u64` and mixed into every random
/// draw a graph makes, so a single pooled graph shared by many hosts produces
/// *different but reproducible* variation per host. Without it, ten notes
/// sharing one "Humanize" graph would humanize identically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HostKey(pub u64);

impl HostKey {
    /// The raw salt value.
    #[inline]
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<PatternId> for HostKey {
    fn from(id: PatternId) -> Self {
        // Rotate into the high half so pattern/note/track id spaces don't alias.
        Self(u64::from(id.0).rotate_left(1))
    }
}

impl From<NoteId> for HostKey {
    fn from(id: NoteId) -> Self {
        Self(id.0.rotate_left(21))
    }
}

impl From<TrackId> for HostKey {
    fn from(id: TrackId) -> Self {
        Self(u64::from(id.0).rotate_left(43))
    }
}

/// Note-scope resolution context, threaded into per-tick source seeding
/// ([`crate::note_processor::seed_source_at_tick`]) so each source note's optional
/// note-scope graph ([`Note::note_graph`]) can be resolved from the project pool
/// and that single note expanded through it (plan §2.1) — per-note articulation,
/// the generalization of the per-note ornament.
///
/// Carries the pool (to resolve a [`NoteGraphId`]) and a pre-allocated scratch
/// buffer for the inner single-note expansion; the outer seed appends the scratch
/// result to its own output. A `None` [`Option<&mut NoteScopeCtx>`] param disables
/// note-scope — used by the inner recursion (a note-scope graph must not
/// re-resolve its own note) and by callers that intentionally skip it (a dangling
/// or empty pool resolves nothing, so the note simply passes through plain).
pub struct NoteScopeCtx<'a> {
    /// The project's note-graph pool, for resolving `Note::note_graph` ids.
    pub pool: &'a [NoteGraph],
    /// Pre-allocated scratch for the single-note inner expansion. RT-safe:
    /// fixed capacity, cleared per note, never grows.
    pub scratch: &'a mut ExpansionBuffer,
}

/// Runtime-only key identifying the origin of an expanded note, for seeded
/// randomness (plan §0.8.1).
///
/// `ExpandedNote` carries no source id or origin tick (its start tick is
/// implicit), so a graph node that wants stable, reorder-robust randomness for
/// generated/expanded notes needs this key threaded alongside the note. Until
/// it is carried on `ExpandedNote`, evaluation falls back to a weaker
/// `tick ⊕ pitch ⊕ slot` key (see [`fallback_note_seed`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoteEventKey {
    /// The source note this event ultimately derives from.
    pub origin_note: NoteId,
    /// The tick the source note started on.
    pub origin_tick: PatternTick,
    /// Which emission of a 1→N expansion this is (chord tone, echo repeat, …).
    pub emit_ordinal: u16,
}

impl NoteEventKey {
    /// Create a new event key.
    #[inline]
    #[must_use]
    pub const fn new(origin_note: NoteId, origin_tick: PatternTick, emit_ordinal: u16) -> Self {
        Self {
            origin_note,
            origin_tick,
            emit_ordinal,
        }
    }

    /// Fold into a `u64` seed contribution. Fields occupy rotated ranges so they
    /// decorrelate before the SplitMix finalizer.
    #[inline]
    #[must_use]
    pub const fn seed(self) -> u64 {
        self.origin_note.0.rotate_left(32)
            ^ (self.origin_tick.0 as u64)
            ^ (self.emit_ordinal as u64).rotate_left(48)
    }
}

/// Fallback per-note seed used until [`NoteEventKey`] is carried on
/// `ExpandedNote`: `tick ⊕ pitch ⊕ slot`. Deterministic but weaker than the
/// origin key under reorder/merge (plan §0.8.1).
///
/// This is a raw fold, *not* a finished random value: every consumer XORs its
/// own salt (host key, node seed, PRNG counter) and then runs the result
/// through the `splitmix64` avalanche before use (`ProbabilityGate`,
/// `NoteScript` `rand()`, `Humanize`, arpeggiator step). The fields occupy
/// bit-disjoint ranges — `tick.0` (u32) in bits 0-31, `pitch` (MIDI 0-127) in
/// 32-38, `slot` (< `MAX_EXPANSION_EVENTS_PER_TICK`) in 40+ — so the fold is
/// injective over its valid inputs; the mixing lives at the point of use.
#[inline]
#[must_use]
pub fn fallback_note_seed(tick: PatternTick, pitch: Pitch, slot: usize) -> u64 {
    u64::from(tick.0) ^ (u64::from(pitch.as_midi()) << 32) ^ ((slot as u64) << 40)
}

/// The three signal types that flow through Note Grid connections, all
/// evaluated at tick rate. `NoteStream` is the linear spine; `Value`/`Gate`
/// are side-input modulation edges from a source node to a downstream node's
/// value input (they do not branch the stream, plan §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum NotePortType {
    /// The per-tick set of expanded notes (the spine).
    NoteStream,
    /// A continuous control scalar, recomputed per tick.
    Value,
    /// A trigger high on specific ticks.
    Gate,
}

impl NotePortType {
    /// Whether this edge carries the `NoteStream` spine (as opposed to a
    /// `Value`/`Gate` modulation side-input).
    #[must_use]
    pub const fn is_stream(self) -> bool {
        matches!(self, Self::NoteStream)
    }
}

/// Max `Value`/`Gate` input ports a single node exposes — the `in1..in4` a
/// [`NoteScriptTransform`] reads, and index 0 is the modulation input on
/// simpler consumers (e.g. a [`ProbabilityGate`] threshold).
pub const MAX_VALUE_INPUTS: usize = 4;

/// The per-tick `Value`/`Gate` scalars a node receives on its modulation input
/// ports, resolved from incoming `Value`/`Gate` edges before the node runs.
/// `None` = that input is unconnected (the node uses its configured default).
/// Stack-allocated and `Copy` — RT-safe, no heap.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModInputs {
    slots: [Option<f32>; MAX_VALUE_INPUTS],
}

impl ModInputs {
    /// Record the value arriving on input port `index` (out-of-range indices are
    /// dropped — connect-time validation keeps `to_input` in range, this is a
    /// belt-and-braces RT guard).
    #[inline]
    fn set(&mut self, index: u8, value: f32) {
        if let Some(slot) = self.slots.get_mut(index as usize) {
            *slot = Some(value);
        }
    }

    /// The value on input port `index`, if connected.
    #[inline]
    #[must_use]
    pub fn get(&self, index: usize) -> Option<f32> {
        self.slots.get(index).copied().flatten()
    }
}

/// A source-independent generator that emits a note on each Euclidean onset —
/// `pulses` hits distributed as evenly as possible over `steps`, offset by
/// `rotation`, one step every `step_len` ticks. Pure tick arithmetic, no state.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
// Missing fields fall back to `Default`, so a partial/empty config (e.g. the MCP
// `add_note_graph_module` `{}`) deserializes like the GUI's `::default()`.
#[serde(default)]
pub struct EuclideanGenerator {
    /// Total steps in the cycle (0 = silent).
    pub steps: u8,
    /// Onsets distributed over the steps (clamped to `steps`).
    pub pulses: u8,
    /// Rotation of the onset pattern, in steps.
    pub rotation: u8,
    /// Ticks per step.
    pub step_len: Duration,
    /// Pitch of each emitted hit.
    pub pitch: Pitch,
    /// Velocity of each emitted hit.
    pub velocity: Velocity,
}

impl Default for EuclideanGenerator {
    fn default() -> Self {
        Self {
            steps: 16,
            pulses: 4,
            rotation: 0,
            // A 16th note at 960 PPQ.
            step_len: Duration(240),
            pitch: Pitch::MIDDLE_C,
            velocity: Velocity::MF,
        }
    }
}

impl EuclideanGenerator {
    /// Whether `tick` lands on a Euclidean onset: a step boundary that is a
    /// pulse under the Bresenham distribution (evenly spreads `pulses` over
    /// `steps`), rotated by `rotation`.
    #[must_use]
    fn is_onset(&self, tick: PatternTick) -> bool {
        let steps = u32::from(self.steps);
        let pulses = u32::from(self.pulses).min(steps);
        if steps == 0
            || pulses == 0
            || self.step_len.0 == 0
            || !tick.0.is_multiple_of(self.step_len.0)
        {
            return false;
        }
        let step = (tick.0 / self.step_len.0) % steps;
        let rotated = (step + u32::from(self.rotation)) % steps;
        (rotated * pulses) % steps < pulses
    }

    /// Append a hit to `buf` when `tick` is an onset. Source-independent — it
    /// ignores whatever the (empty, for a pure generator) upstream left in `buf`.
    fn generate(&self, tick: PatternTick, buf: &mut ExpansionBuffer) {
        if self.is_onset(tick) {
            let _ = buf.push(ExpandedNote {
                duration: Some(self.step_len),
                pitch: self.pitch,
                velocity: self.velocity,
                legato: false,
                glide: None,
                expression: None,
            });
        }
    }
}

/// A pass/block stream filter: each note survives with probability
/// `probability`, decided by a seeded hash (reproducible, host-decorrelated).
/// It rolls its **own** draw rather than writing `NoteExpression::probability`,
/// which the engine resolves on *source* notes only — a generated note's field
/// would never be re-gated (plan §5.D).
///
/// **Not redundant with [`Note::probability`](crate::Note) — they are different
/// layers, do not fold one into the other:**
/// - *This gate* holds **one** probability for its whole scope (optionally
///   modulated per-tick by a `threshold_mod` Value input); it cannot carry a
///   distinct value per note. Its roll keys on *position* (`PatternTick ^ pitch
///   ^ slot`, see [`fallback_note_seed`]) with no loop nonce, so it produces the
///   **same** keep/drop mask every loop pass — a fixed, host-decorrelated
///   pattern.
/// - *Per-note `NoteExpression::probability`* is an authored value carried on
///   each individual note; the engine keys its roll on note **identity** +
///   absolute tick + a loop nonce (`note_passes_probability`), so it **re-rolls
///   each loop pass** and needs no graph. Use it for hand-marked per-note odds.
///
/// Making the gate subsume the per-note field would just move the roll into the
/// graph while keeping the field as its data source — no simplification, and it
/// loses the "works without a graph" property. Keep both.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ProbabilityGate {
    /// Pass probability, 0.0 (block all) .. 1.0 (pass all).
    pub probability: NormalizedValue,
    /// Roll seed — same seed → identical keep/drop decisions.
    pub seed: u64,
}

impl Default for ProbabilityGate {
    fn default() -> Self {
        // Pass-through until configured, so adding one never silently drops.
        Self {
            probability: NormalizedValue::new(1.0),
            seed: 0,
        }
    }
}

impl ProbabilityGate {
    /// Drop notes that fail their seeded roll, in place. The per-note key mixes
    /// the config seed, the host salt, and the fallback note key (§0.8.1), so a
    /// shared graph decorrelates per host and each host is reproducible. A
    /// connected `Value` input (`threshold_mod`) overrides the configured
    /// probability, clamped to `0.0..=1.0`.
    fn apply(
        &self,
        tick: PatternTick,
        host_salt: u64,
        threshold_mod: Option<f32>,
        buf: &mut ExpansionBuffer,
    ) {
        let threshold =
            threshold_mod.map_or_else(|| self.probability.as_f32(), |v| v.clamp(0.0, 1.0));
        if threshold >= 1.0 {
            return; // pass all — no roll needed
        }
        let mut slot = 0usize;
        buf.retain(|note| {
            let key = self.seed ^ host_salt ^ fallback_note_seed(tick, note.pitch, slot);
            slot += 1;
            splitmix64_unit(key) < threshold
        });
    }
}

/// Shape of a [`NoteLfo`]'s unipolar (`0.0..1.0`) waveform over one cycle.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum LfoShape {
    /// Raised sine: `0.5 + 0.5·sin`.
    #[default]
    Sine,
    /// Rising-then-falling triangle.
    Triangle,
    /// Rising ramp (`= phase`).
    Saw,
    /// High for the first half of the cycle, low for the second.
    Square,
}

impl LfoShape {
    /// Unipolar level in `0.0..=1.0` at fractional phase `frac` (`0.0..1.0`).
    #[must_use]
    fn level(self, frac: f32) -> f32 {
        match self {
            Self::Sine => 0.5 + 0.5 * (std::f32::consts::TAU * frac).sin(),
            Self::Triangle => 1.0 - (2.0 * frac - 1.0).abs(),
            Self::Saw => frac,
            Self::Square => {
                if frac < 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }
}

/// A tempo-synced control LFO producing a unipolar `Value` (`0.0..1.0`) from
/// pure tick arithmetic — phase = `f(tick)`, no cross-tick state (plan §5.C).
/// Feeds a downstream node's `Value` input (e.g. a script's `in1`, or a
/// [`ProbabilityGate`] threshold).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct NoteLfo {
    /// Waveform shape.
    pub shape: LfoShape,
    /// Ticks per cycle (0 = frozen at the phase offset).
    pub period: Duration,
    /// Phase offset, `0.0..1.0` of a cycle.
    pub phase: NormalizedValue,
    /// Output depth — scales the `0.0..1.0` level (1.0 = full range).
    pub depth: NormalizedValue,
}

impl Default for NoteLfo {
    fn default() -> Self {
        Self {
            shape: LfoShape::Sine,
            // One cycle per quarter note at 960 PPQ.
            period: Duration(960),
            phase: NormalizedValue::new(0.0),
            depth: NormalizedValue::new(1.0),
        }
    }
}

impl NoteLfo {
    /// Unipolar output at `tick`, scaled by `depth`. Pure function of the tick.
    #[must_use]
    fn value(&self, tick: PatternTick) -> f32 {
        let frac = if self.period.0 == 0 {
            self.phase.as_f32()
        } else {
            let cycle = (tick.0 % self.period.0) as f32 / self.period.0 as f32;
            (cycle + self.phase.as_f32()).fract()
        };
        self.shape.level(frac) * self.depth.as_f32()
    }
}

/// Hard cap on a [`StepLfo`]'s step table (config-side; keeps the value table
/// small and its indexing bounded).
pub const MAX_STEP_LFO_STEPS: usize = 64;

/// A step-sequenced control `Value` source: the output is the current step's
/// level, `step = (tick / step_len) % steps.len()`. Pure tick arithmetic, no
/// cross-tick state (plan §5.C).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct StepLfo {
    /// Per-step output levels (`0.0..1.0`); cycles when exhausted. Empty = 0.
    pub steps: Vec<NormalizedValue>,
    /// Ticks per step (0 = frozen on the first step).
    pub step_len: Duration,
}

impl Default for StepLfo {
    fn default() -> Self {
        Self {
            steps: vec![
                NormalizedValue::new(1.0),
                NormalizedValue::new(0.5),
                NormalizedValue::new(0.75),
                NormalizedValue::new(0.25),
            ],
            // A 16th note at 960 PPQ.
            step_len: Duration(240),
        }
    }
}

impl StepLfo {
    /// The current step's level at `tick`. Pure function of the tick; reads a
    /// bounded slice of the config table (no allocation).
    #[must_use]
    fn value(&self, tick: PatternTick) -> f32 {
        let len = self.steps.len().min(MAX_STEP_LFO_STEPS);
        if len == 0 {
            return 0.0;
        }
        // `checked_div` yields `None` for a zero step length → frozen on step 0.
        let step = tick.0.checked_div(self.step_len.0).unwrap_or(0) as usize % len;
        self.steps[step].as_f32()
    }
}

/// What retriggers a [`NoteEnvelope`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, schemars::JsonSchema,
)]
pub enum EnvelopeTrigger {
    /// Raw source-note onsets — a bounded, pure scan of the pattern notes
    /// (cheap; the default and back-compatible behavior).
    #[default]
    SourceOnset,
    /// Onsets in the *transformed* terminal stream (e.g. one retrigger per
    /// arpeggiator step). Evaluated by a bounded backward look-back through the
    /// spine ([`NoteGraph::eval_prefix_at_tick`]) — more expensive than
    /// `SourceOnset` (each probe re-runs the spine), so it is opt-in **and its
    /// look-back is capped at 1 beat** (`MAX_ENV_STREAM_WINDOW`): more than a
    /// beat after the last transformed onset the level reads `0.0`, even when
    /// `attack + decay` is longer. Retriggers on **dry** transformed onsets
    /// only — a delay's echoes do not retrigger it (RT protection: probing
    /// echoes would compound each in-spine delay's own look-back into an
    /// audio-thread stall). See `note-grid.md` §11.
    StreamOnset,
}

/// An attack/decay envelope `Value` source. Level rises to `peak` over `attack`,
/// falls to zero over `decay`, then holds at zero, retriggered per `trigger`
/// (plan §5.C).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct NoteEnvelope {
    /// Ticks from onset to `peak`.
    pub attack: Duration,
    /// Ticks from `peak` back to zero.
    pub decay: Duration,
    /// Peak level (`0.0..1.0`).
    pub peak: NormalizedValue,
    /// What restarts the envelope (source onsets vs. transformed-stream onsets).
    #[serde(default)]
    pub trigger: EnvelopeTrigger,
}

impl Default for NoteEnvelope {
    fn default() -> Self {
        Self {
            attack: Duration(0),
            decay: Duration(480),
            peak: NormalizedValue::new(1.0),
            trigger: EnvelopeTrigger::SourceOnset,
        }
    }
}

impl NoteEnvelope {
    /// The envelope's active span in ticks (`attack + decay`). `0` means the
    /// envelope is silent. Deliberately uncapped: the `SourceOnset` scan is
    /// bounded by the source-note count, not the window, so a long pad envelope
    /// keeps its full ramp. The `StreamOnset` backward scan applies its own
    /// [`MAX_ENV_STREAM_WINDOW`] cap (it re-runs the spine per probe).
    #[must_use]
    fn window(&self) -> u32 {
        self.attack.0.saturating_add(self.decay.0)
    }

    /// The AD level at `elapsed` ticks after an onset (0 outside the envelope).
    #[must_use]
    fn level_at(&self, elapsed: u32) -> f32 {
        let attack = self.attack.0;
        let decay = self.decay.0;
        let peak = self.peak.as_f32();
        if elapsed < attack {
            peak * (elapsed as f32 / attack as f32)
        } else {
            let into_decay = elapsed - attack;
            if into_decay >= decay {
                // Past the decay tail — silent. A zero-length decay collapses to
                // the instantaneous peak sample at `elapsed == attack` (so the
                // attack ramp still reaches `peak`), then silence.
                if into_decay == 0 { peak } else { 0.0 }
            } else {
                peak * (1.0 - into_decay as f32 / decay as f32)
            }
        }
    }

    /// Envelope level at `tick`, triggered by the most recent *source* onset at
    /// or before `tick` within the envelope's window. Pure, bounded scan over
    /// `source` (no allocation) — no cross-tick state.
    #[must_use]
    fn value(&self, tick: PatternTick, source: &[Note]) -> f32 {
        let window = self.window();
        if window == 0 {
            return 0.0;
        }
        // The most recent onset in `[tick - window, tick]`; ties resolve to the
        // latest onset (a retrigger restarts the envelope). The bound is
        // inclusive so a zero-length-decay peak at `elapsed == window` survives
        // (its `level_at` is 0 for a normal decay tail, so this changes nothing
        // there).
        let mut best: Option<u32> = None;
        for note in source {
            let onset = note.start.0;
            if onset <= tick.0 && tick.0 - onset <= window {
                best = Some(best.map_or(onset, |b| b.max(onset)));
            }
        }
        best.map_or(0.0, |onset| self.level_at(tick.0 - onset))
    }
}

/// A decaying echo (plan §5.A): at tick `t` it
/// keeps the dry stream and appends copies of the onsets that entered this node
/// at ticks `t − k·delay_ticks` (`k = 1..=repeats`), each scaled by `feedback^k`
/// in velocity. Pure look-back — no queue, no cross-tick state. On the spine
/// head the scan reads the seeded source directly; downstream of a transform it
/// re-runs the upstream prefix at each probed tick via
/// [`NoteGraph::eval_prefix_at_tick`]. RT-safe: bounded by `repeats`,
/// [`MAX_NOTE_DELAY_TICKS`], and the buffer cap.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct NoteDelay {
    /// Ticks between successive echoes.
    pub delay_ticks: Duration,
    /// Echoes after the dry note (clamped to [`MAX_DELAY_REPEATS`]).
    pub repeats: u8,
    /// Per-echo velocity multiplier (`0.0` = instant silence, `1.0` = no decay).
    pub feedback: NormalizedValue,
}

impl Default for NoteDelay {
    fn default() -> Self {
        Self {
            // An eighth note at 960 PPQ.
            delay_ticks: Duration(480),
            repeats: 3,
            feedback: NormalizedValue::new(0.6),
        }
    }
}

/// Gain below which a decayed echo/retrigger rounds to silence — one MIDI
/// velocity step. The timing loops stop there: every later repeat is quieter
/// still, and sub-audible events would only waste buffer slots.
const MIN_AUDIBLE_GAIN: f32 = 1.0 / 127.0;

/// The furthest look-back offset a timing node actually probes: the largest
/// `k · step` with `k ≤ count` and `k · step ≤ MAX_NOTE_DELAY_TICKS`. Matches
/// the emit loops' `back > MAX_NOTE_DELAY_TICKS` break exactly, so the freeze
/// walk never extends for echoes that cannot fire (a `step` beyond the cap
/// makes the node emit nothing — its tail is 0, not the cap).
fn lookback_reach(count: u32, step: u32) -> u32 {
    if step == 0 || step > MAX_NOTE_DELAY_TICKS {
        return 0;
    }
    count.min(MAX_NOTE_DELAY_TICKS / step).saturating_mul(step)
}

impl NoteDelay {
    /// Live echo count — clamped; also bounds the per-tick look-back scans.
    #[must_use]
    fn live_repeats(&self) -> u8 {
        self.repeats.min(MAX_DELAY_REPEATS)
    }

    /// Furthest past an onset this node actually reaches — the freeze walk
    /// extends by this so echoes past the pattern end still bake (like
    /// `strum_tail`).
    #[must_use]
    fn delay_tail(&self) -> u32 {
        lookback_reach(u32::from(self.live_repeats()), self.delay_ticks.0)
    }
}

/// A ratchet / roll (timing node, plan §5.A): subdivides each incoming note into
/// `count` fast retriggers spaced `sub_ticks` apart, for as long as the note
/// sounds. Like [`NoteDelay`] it is pure look-back — at tick `t` it re-derives
/// the onsets that entered it at `t − m·sub_ticks` and re-emits those still
/// sounding — but the dry onset is shortened to one subdivision so the note is
/// *replaced* by its stutter rather than layered under it. RT-safe: bounded by
/// `count`, [`MAX_NOTE_DELAY_TICKS`], and the buffer cap.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct Ratchet {
    /// Ticks between retriggers (the subdivision length).
    pub sub_ticks: Duration,
    /// Retriggers per note, including the first (clamped to
    /// [`MAX_RATCHET_SUBDIVISIONS`]). `1` is a no-op passthrough.
    pub count: u8,
    /// Per-retrigger velocity multiplier (`1.0` = no decay).
    pub decay: NormalizedValue,
}

impl Default for Ratchet {
    fn default() -> Self {
        Self {
            // A 32nd note at 960 PPQ.
            sub_ticks: Duration(120),
            count: 4,
            decay: NormalizedValue::new(1.0),
        }
    }
}

impl Ratchet {
    /// Live retrigger count — clamped; also bounds the per-tick look-back scans.
    #[must_use]
    fn live_count(&self) -> u8 {
        self.count.min(MAX_RATCHET_SUBDIVISIONS)
    }

    /// Furthest past an onset a retrigger can actually land — the freeze walk
    /// tail (mirrors [`NoteDelay::delay_tail`]).
    #[must_use]
    fn ratchet_tail(&self) -> u32 {
        lookback_reach(
            u32::from(self.live_count().saturating_sub(1)),
            self.sub_ticks.0,
        )
    }
}

/// Upper bound on a `note_event` script's source registers (note fields + `inN`,
/// counted per reference). A stack scratch of this size feeds `eval_note`.
const MAX_SCRIPT_NOTE_INPUTS: usize = 64;

/// A per-event YAMS `note_event` script transforming the stream 1:1 (plan §6.1).
///
/// Holds the canonical `source` (serialized) and a compiled `BoundScript`
/// (`#[serde(skip)]` — rebuilt from `source` on load / edit by a non-RT sync
/// step, never on the audio thread). With no compiled program the node is
/// pass-through. Pure by construction: the VM is deterministic and per-event
/// state resets.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NoteScriptTransform {
    /// The YAMS `note_event` source text.
    pub source: String,
    /// Compiled program, rebuilt from `source`; not serialized and excluded from
    /// equality (derived state).
    #[serde(skip)]
    #[schemars(skip)]
    compiled: Option<Arc<BoundScript>>,
}

impl PartialEq for NoteScriptTransform {
    fn eq(&self, other: &Self) -> bool {
        // Equality is over the authored source; the compiled program is derived.
        self.source == other.source
    }
}

impl NoteScriptTransform {
    /// A transform with the given source, not yet compiled (pass-through until
    /// [`Self::set_compiled`] installs a program).
    #[must_use]
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            compiled: None,
        }
    }

    /// The canonical YAMS source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether a compiled program is installed (`false` ⇒ pass-through).
    #[must_use]
    pub fn is_compiled(&self) -> bool {
        self.compiled.is_some()
    }

    /// Install (or clear) the compiled program — called from the non-RT compile
    /// step (GUI / MCP / load), never the audio thread.
    pub fn set_compiled(&mut self, script: Option<Arc<BoundScript>>) {
        self.compiled = script;
    }

    /// Run the script over every note in `buf`, 1:1, rewriting fields and
    /// dropping notes per the sentinel rules (plan §6.1). Pass-through when
    /// uncompiled. Each note's PRNG seed mixes `host_salt` with the event's
    /// identity (tick / pitch / slot), so a shared graph decorrelates per host
    /// *and* `rand()` varies per note. RT-safe: no allocation (a `RegisterFile`
    /// is a fixed-size stack struct).
    fn apply(
        &self,
        tick: PatternTick,
        host_salt: u64,
        mods: &ModInputs,
        buf: &mut ExpansionBuffer,
    ) {
        let Some(script) = self.compiled.as_ref() else {
            return; // uncompiled → pass-through
        };
        let mut slot = 0usize;
        buf.retain_mut(|note| {
            // Per-event reset (stateless by construction). The PRNG seed mixes the
            // host key AND the event's identity (tick / pitch / buffer slot), the
            // same idiom as `ProbabilityGate`, so a `rand()` in the script varies
            // per note rather than returning one per-host constant.
            let seed = host_salt ^ fallback_note_seed(tick, note.pitch, slot);
            slot += 1;
            let mut regs = RegisterFile::new(0, seed);
            let mut sources = [0.0f32; MAX_SCRIPT_NOTE_INPUTS];
            let n = script.inputs.len().min(MAX_SCRIPT_NOTE_INPUTS);
            for (slot, input) in sources.iter_mut().zip(script.inputs.iter()).take(n) {
                *slot = note_source_value(input, note, tick, mods);
            }
            let out = script.script.eval_note(&sources[..n], &mut regs);
            apply_note_outputs(note, &out)
        });
    }
}

/// Resolve one `note_event` script input register for a note event.
fn note_source_value(
    input: &ScriptInput,
    note: &ExpandedNote,
    tick: PatternTick,
    mods: &ModInputs,
) -> f32 {
    match input {
        ScriptInput::NoteField(NoteField::Pitch) => f32::from(note.pitch.as_midi()),
        ScriptInput::NoteField(NoteField::Vel) => note.velocity.as_f32(),
        ScriptInput::NoteField(NoteField::Dur) => note.duration.map_or(-1.0, |d| d.0 as f32),
        ScriptInput::NoteField(NoteField::Tick) => tick.0 as f32,
        // An unconnected value input reads 0.
        ScriptInput::NoteInput(k) => mods.get(*k as usize).unwrap_or(0.0),
        _ => 0.0,
    }
}

/// Apply the script's outputs to a note — sentinel checks first, then clamp
/// (plan §6.1) — returning `false` to drop the note. Unwritten outputs (`None`)
/// pass the field through unchanged.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn apply_note_outputs(note: &mut ExpandedNote, out: &NoteOutputs) -> bool {
    // Sentinels run before clamping so a clamp cannot erase them.
    if let Some(v) = out.vel {
        if v < 0.0 {
            return false; // negative velocity drops the note
        }
        note.velocity = Velocity::new(v.clamp(0.0, 1.0));
    }
    if let Some(d) = out.dur {
        note.duration = if d < 0.0 {
            None // negative duration restores "plays until cut"
        } else {
            Some(Duration(d.round().max(0.0) as u32))
        };
    }
    if let Some(p) = out.pitch
        && let Some(pitch) = Pitch::new(p.round().clamp(0.0, 127.0) as u8)
    {
        note.pitch = pitch;
    }
    // Gate scales the (post-`out_dur`) duration — the expanded stream carries no
    // separate gate field, matching how the rack shortens a note.
    if let Some(g) = out.gate
        && let Some(d) = note.duration
    {
        note.duration = Some(Duration((d.0 as f32 * g.clamp(0.0, 1.0)).round() as u32));
    }
    true
}

/// Static configuration of one graph node — the pattern-rack DSP or a native
/// Note Grid module.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum NoteModuleConfig {
    /// A stream transform/generator reusing the pattern rack's DSP.
    Processor(NoteProcessor),
    /// A Euclidean rhythm generator (source-independent).
    Euclidean(EuclideanGenerator),
    /// A seeded pass/block probability filter.
    ProbabilityGate(ProbabilityGate),
    /// A tempo-synced control LFO (`Value` source).
    NoteLfo(NoteLfo),
    /// A step-sequenced control LFO (`Value` source).
    StepLfo(StepLfo),
    /// An attack/decay envelope retriggered by source onsets (`Value` source).
    NoteEnvelope(NoteEnvelope),
    /// A per-event YAMS `note_event` script (1-in/1-out stream transform).
    NoteScriptTransform(NoteScriptTransform),
    /// A decaying echo (timing node with bounded look-back, plan §5.A).
    NoteDelay(NoteDelay),
    /// A ratchet / roll: subdivides each note into fast retriggers (timing node).
    Ratchet(Ratchet),
}

impl NoteModuleConfig {
    /// Stable snake_case tag for readers / MCP.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Processor(p) => p.kind(),
            Self::Euclidean(_) => "euclidean",
            Self::ProbabilityGate(_) => "probability_gate",
            Self::NoteLfo(_) => "note_lfo",
            Self::StepLfo(_) => "step_lfo",
            Self::NoteEnvelope(_) => "note_envelope",
            Self::NoteScriptTransform(_) => "note_script_transform",
            Self::NoteDelay(_) => "note_delay",
            Self::Ratchet(_) => "ratchet",
        }
    }

    /// The user-facing node name, shared by the node header and the add-node
    /// menu. One definition per kind, so the palette entry and the node title
    /// can't disagree (they once read "Euclidean Generator" and "Euclidean").
    #[must_use]
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Processor(p) => p.display_name(),
            Self::Euclidean(_) => "Euclidean",
            Self::ProbabilityGate(_) => "Probability Gate",
            Self::NoteLfo(_) => "Note LFO",
            Self::StepLfo(_) => "Step LFO",
            Self::NoteEnvelope(_) => "Note Envelope",
            Self::NoteScriptTransform(_) => "Script",
            Self::NoteDelay(_) => "Delay / Echo",
            Self::Ratchet(_) => "Ratchet",
        }
    }

    /// Whether this node consumes an upstream `NoteStream`. Pure generators
    /// (Euclidean) and `Value` sources (LFOs, envelope) do not.
    #[must_use]
    pub const fn has_stream_input(&self) -> bool {
        match self {
            Self::Processor(_)
            | Self::ProbabilityGate(_)
            | Self::NoteScriptTransform(_)
            | Self::NoteDelay(_)
            | Self::Ratchet(_) => true,
            Self::Euclidean(_) | Self::NoteLfo(_) | Self::StepLfo(_) | Self::NoteEnvelope(_) => {
                false
            }
        }
    }

    /// Whether this node emits a `NoteStream`. `Value` sources do not.
    #[must_use]
    pub const fn has_stream_output(&self) -> bool {
        match self {
            Self::Processor(_)
            | Self::Euclidean(_)
            | Self::ProbabilityGate(_)
            | Self::NoteScriptTransform(_)
            | Self::NoteDelay(_)
            | Self::Ratchet(_) => true,
            Self::NoteLfo(_) | Self::StepLfo(_) | Self::NoteEnvelope(_) => false,
        }
    }

    /// Whether this node produces a `Value`/`Gate` scalar (a modulation source)
    /// rather than operating on the `NoteStream`.
    #[must_use]
    pub const fn is_value_source(&self) -> bool {
        matches!(
            self,
            Self::NoteLfo(_) | Self::StepLfo(_) | Self::NoteEnvelope(_)
        )
    }

    /// How many `Value` input ports this node exposes (0 if it takes no
    /// modulation). Used by connect-time validation to range-check `to_input`.
    #[must_use]
    pub const fn value_input_count(&self) -> u8 {
        match self {
            // Threshold modulation on port 0.
            Self::ProbabilityGate(_) => 1,
            // `in1`..`in4` map to the four value input ports.
            Self::NoteScriptTransform(_) => MAX_VALUE_INPUTS as u8,
            Self::Processor(_)
            | Self::Euclidean(_)
            | Self::NoteLfo(_)
            | Self::StepLfo(_)
            | Self::NoteEnvelope(_)
            | Self::NoteDelay(_)
            | Self::Ratchet(_) => 0,
        }
    }

    /// Evaluate this node at `tick`. A `Value` source returns `Some(scalar)`
    /// and leaves `buf` untouched; a stream node returns `None` after applying
    /// itself to the working `NoteStream` buffer, reading any modulation on
    /// `mods`. `ctx` carries the source-context a node may consult (raw notes,
    /// tempo, upstream held-pitch view). `host_salt` mixes into seeded
    /// randomness. RT-safe: no allocation.
    pub(crate) fn eval(
        &self,
        tick: PatternTick,
        host_salt: u64,
        ctx: &SourceCtx,
        mods: &ModInputs,
        buf: &mut ExpansionBuffer,
    ) -> Option<f32> {
        match self {
            // Source-independent processors run in place; the source-context
            // ones (Arp / strummed Chord) read `ctx.source` + `ctx.upstream`.
            Self::Processor(p) => {
                p.process_at_tick(ctx.source, tick, ctx.upstream, ctx.bpm, host_salt, buf);
                None
            }
            Self::Euclidean(e) => {
                e.generate(tick, buf);
                None
            }
            Self::ProbabilityGate(g) => {
                g.apply(tick, host_salt, mods.get(0), buf);
                None
            }
            Self::NoteLfo(l) => Some(l.value(tick)),
            Self::StepLfo(s) => Some(s.value(tick)),
            Self::NoteEnvelope(e) => Some(e.value(tick, ctx.source)),
            Self::NoteScriptTransform(t) => {
                t.apply(tick, host_salt, mods, buf);
                None
            }
            // Timing nodes are dispatched by `expand_impl`'s look-back branch
            // (they need the graph + spine position + scratch pool), never the
            // stateless `eval` path — these arms are a defensive pass-through.
            Self::NoteDelay(_) | Self::Ratchet(_) => None,
        }
    }
}

/// The source-context a node's [`NoteModuleConfig::eval`] may consult: the
/// pattern's raw notes, the tempo (frame-locked arp rates), and the
/// spine-prefix processors that define the held-pitch view (the 1→N seam).
/// Bundled so the per-tick eval signature stays small. RT-safe: all borrows,
/// no allocation.
pub(crate) struct SourceCtx<'a> {
    pub source: &'a [Note],
    pub bpm: Bpm,
    pub upstream: &'a [NoteProcessor],
}

/// A directed connection between two nodes' ports of a given signal type.
///
/// v1 stores only `NoteStream` links (the linear spine). The type is carried so
/// the DAG escalation can add `Value`/`Gate` edges without a format change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NoteConnection {
    /// Source node (output side).
    pub from: NoteModuleId,
    /// Destination node (input side).
    pub to: NoteModuleId,
    /// Signal type carried by this edge.
    pub port: NotePortType,
    /// For `Value`/`Gate` edges, which of the target's value input ports this
    /// feeds (`0..value_input_count`). Ignored for `NoteStream` edges.
    #[serde(default)]
    pub to_input: u8,
}

impl NoteConnection {
    /// Create a `NoteStream` connection (the v1 spine edge).
    #[must_use]
    pub const fn stream(from: NoteModuleId, to: NoteModuleId) -> Self {
        Self {
            from,
            to,
            port: NotePortType::NoteStream,
            to_input: 0,
        }
    }

    /// Create a `Value` modulation edge into the target's `to_input` port.
    #[must_use]
    pub const fn value(from: NoteModuleId, to: NoteModuleId, to_input: u8) -> Self {
        Self {
            from,
            to,
            port: NotePortType::Value,
            to_input,
        }
    }

    /// Create a `Gate` modulation edge into the target's `to_input` port.
    #[must_use]
    pub const fn gate(from: NoteModuleId, to: NoteModuleId, to_input: u8) -> Self {
        Self {
            from,
            to,
            port: NotePortType::Gate,
            to_input,
        }
    }
}

/// Hard cap on nodes in one graph (plan §1.4). Adding past it is a non-RT-side
/// error, never an audio-thread concern.
pub const MAX_NOTE_GRID_NODES: usize = 32;

/// Errors from editing / validating a [`NoteGraph`]. Non-RT side only — the
/// audio thread never sees an invalid or cyclic graph because these are
/// rejected at edit time.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoteGraphError {
    /// Adding a node would exceed [`MAX_NOTE_GRID_NODES`].
    #[error("note graph is at the {MAX_NOTE_GRID_NODES}-node cap")]
    NodeCapExceeded,
    /// A connection references a node id not present in the graph.
    #[error("connection references unknown node {0:?}")]
    UnknownNode(NoteModuleId),
    /// A node would receive two `NoteStream` inputs (merge is a DAG feature).
    #[error("node {0:?} would have two NoteStream inputs (merge not allowed in v1)")]
    DuplicateStreamInput(NoteModuleId),
    /// A node would feed two `NoteStream` outputs (split is a DAG feature).
    #[error("node {0:?} would have two NoteStream outputs (split not allowed in v1)")]
    DuplicateStreamOutput(NoteModuleId),
    /// A `NoteStream` edge attaches to a node lacking the required stream port
    /// (e.g. a `Value` source has neither a stream input nor a stream output).
    #[error("node {0:?} has no NoteStream port for this connection")]
    InvalidStreamEndpoint(NoteModuleId),
    /// A `Value`/`Gate` edge originates at a node that produces no modulation
    /// signal (only `Value` sources like `NoteLfo` can).
    #[error("node {0:?} is not a Value/Gate source")]
    NotValueSource(NoteModuleId),
    /// A `Value`/`Gate` edge targets a node/port that accepts no such input
    /// (`to_input` out of range, or the node takes no modulation).
    #[error("node {0:?} has no Value input port {1}")]
    InvalidValueInput(NoteModuleId, u8),
    /// The exact same connection exists twice.
    #[error("connection {0:?} → {1:?} already exists")]
    DuplicateConnection(NoteModuleId, NoteModuleId),
    /// Two `Value`/`Gate` edges feed the same input port — evaluation would
    /// silently let the last edge win, so a second source is rejected.
    #[error("node {0:?} Value input {1} already has a source")]
    DuplicateValueInput(NoteModuleId, u8),
    /// The connection set contains a cycle (spans all edge types, §0.8.4).
    #[error("connection would create a cycle")]
    Cycle,
}

/// A node's canvas position in the Note Grid editor (world coordinates).
/// Layout metadata only — evaluation ignores it; a node without one gets
/// auto-layout.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NodePosition {
    pub x: f32,
    pub y: f32,
}

/// Static note-graph configuration — a pooled project asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NoteGraph {
    /// Pool-unique id.
    pub id: NoteGraphId,
    /// User-facing name, shown in the pool view.
    pub name: String,
    /// Free-text description.
    #[serde(default)]
    pub description: String,
    /// Sequencer-side color (RGB), never egui's `Color32` — this crate has no
    /// egui dependency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<TrackColor>,
    /// Nodes keyed by graph-local id.
    #[serde(default)]
    pub nodes: BTreeMap<NoteModuleId, NoteModuleConfig>,
    /// Pedagogical/user intent per node, kept separate from DSP configs.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_descriptions: BTreeMap<NoteModuleId, String>,
    /// Connections, kept as a sorted `Vec` for deterministic serialization and
    /// iteration. v1: `NoteStream` axis validated linear; `Value`/`Gate` edges
    /// permitted once modulators exist.
    #[serde(default)]
    pub connections: Vec<NoteConnection>,
    /// Editor canvas positions per node (layout metadata; nodes without one
    /// are auto-laid-out). Persisted so an arranged canvas survives reload.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub node_positions: BTreeMap<NoteModuleId, NodePosition>,
    /// Derived evaluation order — NOT serialized. Rebuilt by
    /// [`NoteGraph::rebuild_derived`] on every non-RT load/mutation path; never
    /// recomputed on the audio thread.
    #[serde(skip)]
    pub processing_order: Vec<NoteModuleId>,
    /// Derived active `NoteStream` chain, head → terminal — NOT serialized.
    /// Stream nodes outside it are inert at evaluation; see
    /// [`NoteGraph::rebuild_stream_spine`]. Rebuilt with `processing_order`.
    #[serde(skip)]
    pub stream_spine: Vec<NoteModuleId>,
}

impl NoteGraph {
    /// Create an empty graph with the given id and name.
    #[must_use]
    pub fn new(id: NoteGraphId, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            description: String::new(),
            color: None,
            nodes: BTreeMap::new(),
            node_descriptions: BTreeMap::new(),
            connections: Vec::new(),
            node_positions: BTreeMap::new(),
            processing_order: Vec::new(),
            stream_spine: Vec::new(),
        }
    }

    /// Number of nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// The smallest unused [`NoteModuleId`] — used by non-RT editors (GUI / MCP)
    /// to allocate node ids. Smallest-unused rather than `max + 1` so a restored
    /// graph containing a `u32::MAX` node can't saturate into a collision (which
    /// would silently overwrite that node). Bounded by `node_count + 1`
    /// iterations: one of `0..=node_count` is always free (pigeonhole), and the
    /// graph is node-capped well below `u32::MAX`.
    pub fn next_module_id(&self) -> NoteModuleId {
        let mut candidate = 0u32;
        while self.nodes.contains_key(&NoteModuleId::new(candidate)) {
            candidate += 1;
        }
        NoteModuleId::new(candidate)
    }

    /// Mutable iterator over every node config, in id order. For a non-RT
    /// resync step that rebuilds derived per-node state which is not serialized
    /// — e.g. recompiling each [`NoteScriptTransform`]'s program from its
    /// `source` after a load. Structural edits must still go through
    /// [`Self::try_insert_node`] / [`Self::remove_node`] (which maintain the
    /// derived order); this is for in-place field updates only.
    pub fn nodes_mut(&mut self) -> impl Iterator<Item = &mut NoteModuleConfig> {
        self.nodes.values_mut()
    }

    /// Remove a node and every connection touching it, then recompute the
    /// processing order (dropping edges can never introduce a cycle or a
    /// linearity violation, so this cannot fail). Returns the removed config, or
    /// `None` if the id was absent. Non-RT side only.
    pub fn remove_node(&mut self, id: NoteModuleId) -> Option<NoteModuleConfig> {
        let removed = self.nodes.remove(&id)?;
        self.connections.retain(|c| c.from != id && c.to != id);
        self.node_positions.remove(&id);
        self.node_descriptions.remove(&id);
        // Removing nodes/edges only relaxes constraints — rebuild is infallible.
        let _ = self.rebuild_derived();
        Some(removed)
    }

    /// The terminal `NoteStream` node — the one that feeds the host instrument:
    /// the last node of the [active spine](Self::rebuild_stream_spine). `None`
    /// for a graph with no stream-emitting nodes.
    #[must_use]
    pub fn stream_output_node(&self) -> Option<NoteModuleId> {
        self.stream_spine.last().copied()
    }

    /// Whether the active spine is headed by a pure generator — its first node
    /// emits a `NoteStream` but has no stream *input* (e.g.
    /// [`EuclideanGenerator`]). Such a spine defines its own stream and ignores
    /// the pattern's source notes; a spine headed by a transform (quantize,
    /// gate, humanize) instead processes the seeded source in place.
    #[must_use]
    fn stream_head_is_generator(&self) -> bool {
        self.stream_spine
            .first()
            .and_then(|id| self.nodes.get(id))
            .is_some_and(|c| !c.has_stream_input())
    }

    /// Recompute [`Self::stream_spine`] — the single active `NoteStream` chain,
    /// walked upstream from its terminal. When unwired stream nodes leave
    /// several candidate chains, the **longest** wins (ties: smallest terminal
    /// id), so dropping a fresh, still-unwired node onto the canvas never
    /// hijacks the graph's output. Nodes off the spine are inert at evaluation
    /// (like an unwired module in the patch editor). Non-RT side only.
    fn rebuild_stream_spine(&mut self) {
        let has_stream_consumer: BTreeSet<NoteModuleId> = self
            .connections
            .iter()
            .filter(|c| c.port == NotePortType::NoteStream)
            .map(|c| c.from)
            .collect();
        let mut best: Vec<NoteModuleId> = Vec::new();
        // Terminals in ascending id order (BTreeMap), so on equal length the
        // smallest terminal id is kept (strictly-greater comparison below).
        for (&terminal, config) in &self.nodes {
            if !config.has_stream_output() || has_stream_consumer.contains(&terminal) {
                continue;
            }
            // Walk upstream via the ≤1-stream-input linearity guarantee.
            let mut chain = vec![terminal];
            let mut current = terminal;
            while let Some(edge) = self
                .connections
                .iter()
                .find(|c| c.port.is_stream() && c.to == current)
            {
                current = edge.from;
                chain.push(current);
            }
            if chain.len() > best.len() {
                chain.reverse();
                best = chain;
            }
        }
        self.stream_spine = best;
    }

    /// Insert (or replace) a node, enforcing the [`MAX_NOTE_GRID_NODES`] cap
    /// (replacing an existing id never trips the cap) and recomputing the
    /// processing order so the node takes effect immediately.
    pub fn try_insert_node(
        &mut self,
        id: NoteModuleId,
        config: NoteModuleConfig,
    ) -> Result<(), NoteGraphError> {
        if !self.nodes.contains_key(&id) && self.nodes.len() >= MAX_NOTE_GRID_NODES {
            return Err(NoteGraphError::NodeCapExceeded);
        }
        let previous = self.nodes.insert(id, config);
        // Recompute the order so the new node is included even without a
        // following `try_connect` (a lone node must still expand). Adding a
        // *new* node introduces no edges, but *replacing* one can invalidate an
        // existing modulation edge (e.g. swapping a `Value` source for a stream
        // node), so roll back to the prior state on rejection — like
        // `try_connect` — leaving the graph unchanged.
        if let Err(e) = self.rebuild_derived() {
            match previous {
                Some(old) => {
                    self.nodes.insert(id, old);
                }
                None => {
                    self.nodes.remove(&id);
                }
            }
            let _ = self.rebuild_derived();
            return Err(e);
        }
        Ok(())
    }

    /// Add a connection, validating linearity + acyclicity, then recompute the
    /// processing order. On error the connection is rolled back and the previous
    /// (valid) order is restored.
    pub fn try_connect(&mut self, connection: NoteConnection) -> Result<(), NoteGraphError> {
        self.connections.push(connection);
        if let Err(e) = self.rebuild_derived() {
            self.connections.pop();
            // Restore the order that held before the rejected edit.
            let _ = self.rebuild_derived();
            return Err(e);
        }
        Ok(())
    }

    /// Repair connection-set shapes that were legal before a validation rule
    /// existed, so older project files keep loading playable (§0.8.3 spirit):
    /// exact duplicate edges collapse to one, and of several `Value`/`Gate`
    /// edges into the same input port only the **last** survives — matching
    /// the old evaluator's last-edge-wins resolution, so a sanitized legacy
    /// graph sounds exactly as it did when saved. Load-path only; live edits
    /// are rejected up front by [`Self::rebuild_derived`] instead.
    pub fn sanitize_connections(&mut self) {
        let mut taken_value_in: BTreeSet<(NoteModuleId, u8)> = BTreeSet::new();
        let mut kept_rev: Vec<NoteConnection> = Vec::with_capacity(self.connections.len());
        // Walk backwards so "first seen" = "last in file order".
        for c in self.connections.iter().rev() {
            if kept_rev.contains(c) {
                continue;
            }
            if !c.port.is_stream() && !taken_value_in.insert((c.to, c.to_input)) {
                continue;
            }
            kept_rev.push(*c);
        }
        kept_rev.reverse();
        self.connections = kept_rev;
    }

    /// Validate the connection set and recompute [`Self::processing_order`] by a
    /// Kahn topological sort. **Non-RT side only** — call on every UI / MCP /
    /// headless load or mutation before releasing the `Song` write lock; never
    /// on the audio thread. On error, `processing_order` is left unchanged and
    /// the caller must treat the graph as unplayable until fixed.
    ///
    /// v1 rules: every connection endpoint must exist; the `NoteStream` axis is
    /// linear (each node ≤ 1 stream input and ≤ 1 stream output); and the whole
    /// edge set (all port types) must be acyclic.
    pub fn rebuild_derived(&mut self) -> Result<(), NoteGraphError> {
        // 1. Endpoints must reference existing nodes, and the NoteStream axis
        //    must stay linear. Build the indegree + adjacency maps for Kahn in
        //    the same pass.
        let mut indegree: BTreeMap<NoteModuleId, usize> =
            self.nodes.keys().map(|&id| (id, 0)).collect();
        let mut adjacency: BTreeMap<NoteModuleId, Vec<NoteModuleId>> = BTreeMap::new();
        let mut stream_out: BTreeMap<NoteModuleId, u32> = BTreeMap::new();
        let mut stream_in: BTreeMap<NoteModuleId, u32> = BTreeMap::new();
        let mut value_in: BTreeSet<(NoteModuleId, u8)> = BTreeSet::new();

        for (i, c) in self.connections.iter().enumerate() {
            // Exact duplicates would double-draw and double-count everywhere
            // downstream; O(n²) over the node-capped edge set is fine here.
            if self.connections[..i].contains(c) {
                return Err(NoteGraphError::DuplicateConnection(c.from, c.to));
            }
            if !self.nodes.contains_key(&c.from) {
                return Err(NoteGraphError::UnknownNode(c.from));
            }
            if !self.nodes.contains_key(&c.to) {
                return Err(NoteGraphError::UnknownNode(c.to));
            }
            if c.port.is_stream() {
                // Both endpoints must actually expose a stream port — a `Value`
                // source has neither, so it can never sit on the spine.
                if !self.nodes[&c.from].has_stream_output() {
                    return Err(NoteGraphError::InvalidStreamEndpoint(c.from));
                }
                if !self.nodes[&c.to].has_stream_input() {
                    return Err(NoteGraphError::InvalidStreamEndpoint(c.to));
                }
                let out = stream_out.entry(c.from).or_default();
                *out += 1;
                if *out > 1 {
                    return Err(NoteGraphError::DuplicateStreamOutput(c.from));
                }
                let inp = stream_in.entry(c.to).or_default();
                *inp += 1;
                if *inp > 1 {
                    return Err(NoteGraphError::DuplicateStreamInput(c.to));
                }
            } else {
                // A `Value`/`Gate` edge: source must produce modulation, target
                // must expose the addressed input port. Cycles that run through
                // these edges are still caught by the Kahn sort below (§0.8.4).
                if !self.nodes[&c.from].is_value_source() {
                    return Err(NoteGraphError::NotValueSource(c.from));
                }
                if c.to_input >= self.nodes[&c.to].value_input_count() {
                    return Err(NoteGraphError::InvalidValueInput(c.to, c.to_input));
                }
                // One source per input port: evaluation resolves inputs by a
                // linear scan where the last edge would silently win, so a
                // second source is rejected rather than shadowed.
                if !value_in.insert((c.to, c.to_input)) {
                    return Err(NoteGraphError::DuplicateValueInput(c.to, c.to_input));
                }
            }
            adjacency.entry(c.from).or_default().push(c.to);
            *indegree.entry(c.to).or_default() += 1;
        }

        // 2. Kahn topological sort. A BTreeSet ready-queue makes the order
        //    deterministic (smallest NoteModuleId first) for stable diffs.
        let mut ready: BTreeSet<NoteModuleId> = indegree
            .iter()
            .filter_map(|(&id, &deg)| (deg == 0).then_some(id))
            .collect();
        let mut order: Vec<NoteModuleId> = Vec::with_capacity(self.nodes.len());
        while let Some(&node) = ready.iter().next() {
            ready.remove(&node);
            order.push(node);
            if let Some(successors) = adjacency.get(&node) {
                for &succ in successors {
                    if let Some(deg) = indegree.get_mut(&succ) {
                        *deg -= 1;
                        if *deg == 0 {
                            ready.insert(succ);
                        }
                    }
                }
            }
        }

        if order.len() != self.nodes.len() {
            return Err(NoteGraphError::Cycle);
        }
        self.processing_order = order;
        self.rebuild_stream_spine();
        Ok(())
    }

    /// Pure per-tick expansion of `source` through the graph's linear chain,
    /// decorrelated by `host`. The result lands in `out` (cleared first). An
    /// empty / node-less graph passes the source onsets through unchanged.
    /// `gate` filters source onsets — the engine's per-note probability roll,
    /// mirroring the rack; pass `|_| true` for preview / bake.
    ///
    /// Pure function of `(config, source, tick, host)` — no cross-tick state —
    /// which is exactly what the seek, freeze, and preview paths rely on
    /// (plan §1.1). `out` doubles as the working buffer flowing down the chain.
    ///
    /// v1 wraps the pattern-*independent* processors; nodes that need source
    /// context (`Arpeggiator`, strummed `Chord`) are pass-through here and are
    /// wired in a later phase. RT-safe for the wrapped processors: no allocation.
    ///
    /// Only stream nodes on the **active spine** (the single chain feeding
    /// [`Self::stream_output_node`]) process the working buffer — an unwired
    /// stream node is inert, like an unwired module in the patch editor.
    /// `Value` sources always evaluate (side-effect-free scalars); consumers
    /// only read them through explicit edges.
    #[allow(clippy::too_many_arguments)]
    pub fn expand_at_tick(
        &self,
        source: &[Note],
        tick: PatternTick,
        host: HostKey,
        bpm: Bpm,
        gate: impl Fn(&Note) -> bool,
        note_scope: Option<&mut NoteScopeCtx<'_>>,
        lookback: Option<&mut [ExpansionBuffer]>,
        out: &mut ExpansionBuffer,
    ) {
        self.expand_impl(
            source,
            tick,
            host,
            bpm,
            &gate,
            None,
            note_scope,
            lookback.unwrap_or(&mut []),
            false,
            out,
        );
    }

    /// As [`Self::expand_at_tick`] but stops after the spine node `tap`
    /// (inclusive), leaving `out` holding that node's cumulative output — the
    /// tracker's per-node tap (plan §7), a non-RT display path that never gates
    /// probability (like the rack's per-stage columns). A `tap` off the active
    /// spine yields the full expansion (the break is never reached).
    #[allow(clippy::too_many_arguments)]
    pub fn expand_at_tick_tapped(
        &self,
        source: &[Note],
        tick: PatternTick,
        host: HostKey,
        bpm: Bpm,
        tap: NoteModuleId,
        note_scope: Option<&mut NoteScopeCtx<'_>>,
        lookback: Option<&mut [ExpansionBuffer]>,
        out: &mut ExpansionBuffer,
    ) {
        self.expand_impl(
            source,
            tick,
            host,
            bpm,
            &|_| true,
            Some(tap),
            note_scope,
            lookback.unwrap_or(&mut []),
            false,
            out,
        );
    }

    /// The shared per-tick expansion body. `tap`, when set, stops the walk after
    /// that node so the tracker can read an intermediate stage. `pool` is the
    /// scratch + recursion budget for timing nodes' bounded look-back (empty ⇒
    /// timing nodes emit their dry signal only). `gate` is `&dyn` on purpose:
    /// this body re-enters itself through [`Self::eval_prefix_at_tick`], and a
    /// generic gate would make that recursion a monomorphization cycle.
    #[allow(clippy::too_many_arguments)]
    fn expand_impl(
        &self,
        source: &[Note],
        tick: PatternTick,
        host: HostKey,
        bpm: Bpm,
        gate: &dyn Fn(&Note) -> bool,
        tap: Option<NoteModuleId>,
        mut note_scope: Option<&mut NoteScopeCtx<'_>>,
        pool: &mut [ExpansionBuffer],
        in_lookback: bool,
        out: &mut ExpansionBuffer,
    ) {
        // Seed the working buffer, then run the chain in topological order.
        self.seed_stream_input(source, tick, bpm, gate, note_scope.as_deref_mut(), out);
        // Per-tick `Value` source outputs, filled as topological order reaches
        // each source (guaranteed before any consumer). A small linear map keyed
        // by node id — capped at `MAX_NOTE_GRID_NODES`, so stack-only, RT-safe.
        let mut value_slots: [(NoteModuleId, f32); MAX_NOTE_GRID_NODES] =
            [(NoteModuleId::new(0), 0.0); MAX_NOTE_GRID_NODES];
        let mut n_slots = 0usize;
        // Reused across nodes (initialized once per tick, not per node): the
        // spine-prefix processors a source-context node views its held pitches
        // through. Only `up[..up_n]` is read, so stale tail entries never leak.
        let mut up = [NoteProcessor::ScaleQuantize(Default::default()); MAX_NOTE_GRID_NODES];
        for id in &self.processing_order {
            let Some(config) = self.nodes.get(id) else {
                continue;
            };
            // Off-spine stream nodes are inert (Value sources fall through —
            // they never touch the buffer). One linear scan of a ≤32-entry Vec
            // yields both the membership check and the spine position the
            // timing nodes need: RT-safe, no allocation.
            let spine_pos = if config.is_value_source() {
                None
            } else {
                match self.stream_spine.iter().position(|x| x == id) {
                    Some(pos) => Some(pos),
                    None => continue,
                }
            };
            // Timing nodes keep the dry stream and append look-back echoes; they
            // need the graph + spine position + scratch pool, so they bypass the
            // stateless `eval` path (whose exhaustive match is what forces a
            // future timing variant to decide where it dispatches). Non-timing
            // configs fall through to the generic path below. `spine_pos` is
            // `Some` here (timing nodes are stream nodes and passed the check);
            // `unwrap_or(0)` is the panic-free fallback the audio thread requires.
            match config {
                NoteModuleConfig::NoteDelay(delay) => {
                    self.apply_delay(
                        delay,
                        spine_pos.unwrap_or(0),
                        tick,
                        source,
                        host,
                        bpm,
                        gate,
                        note_scope.as_deref_mut(),
                        pool,
                        out,
                    );
                    if tap == Some(*id) {
                        break;
                    }
                    continue;
                }
                NoteModuleConfig::Ratchet(ratchet) => {
                    self.apply_ratchet(
                        ratchet,
                        spine_pos.unwrap_or(0),
                        tick,
                        source,
                        host,
                        bpm,
                        gate,
                        note_scope.as_deref_mut(),
                        pool,
                        out,
                    );
                    if tap == Some(*id) {
                        break;
                    }
                    continue;
                }
                _ => {}
            }
            // Resolve modulation inputs from already-computed upstream sources.
            let mut mods = ModInputs::default();
            if config.value_input_count() > 0 {
                for c in &self.connections {
                    if c.to == *id
                        && !c.port.is_stream()
                        && let Some(&(_, v)) = value_slots[..n_slots]
                            .iter()
                            .find(|(sid, _)| sid == &c.from)
                    {
                        mods.set(c.to_input, v);
                    }
                }
            }
            // Source-context processors (arp / strummed chord) view their held
            // pitches through the spine-prefix processors (RT-safe: `NoteProcessor`
            // is `Copy`, capped at `MAX_NOTE_GRID_NODES`). Non-`Processor` spine
            // nodes do not transform held-source pitch, so they are skipped;
            // other nodes get an empty upstream.
            let mut up_n = 0;
            if matches!(config, NoteModuleConfig::Processor(_)) {
                for sid in &self.stream_spine {
                    if sid == id {
                        break;
                    }
                    if let Some(NoteModuleConfig::Processor(up_p)) = self.nodes.get(sid)
                        && up_n < up.len()
                    {
                        up[up_n] = *up_p;
                        up_n += 1;
                    }
                }
            }
            let ctx = SourceCtx {
                source,
                bpm,
                upstream: &up[..up_n],
            };
            // A `StreamOnset` envelope retriggers off the *transformed* terminal
            // stream, not raw source — a bounded backward look-back. Only at the
            // top level: during a prefix re-eval (`in_lookback`) it falls back to
            // the cheap source-onset `eval` so it can never recurse into itself.
            let value = match config {
                NoteModuleConfig::NoteEnvelope(env)
                    if env.trigger == EnvelopeTrigger::StreamOnset && !in_lookback =>
                {
                    Some(self.envelope_stream_value(
                        env,
                        tick,
                        source,
                        host,
                        bpm,
                        gate,
                        note_scope.as_deref_mut(),
                        pool,
                    ))
                }
                _ => config.eval(tick, host.get(), &ctx, &mods, out),
            };
            if let Some(value) = value
                && n_slots < value_slots.len()
            {
                value_slots[n_slots] = (*id, value);
                n_slots += 1;
            }
            // Per-node tracker tap: stop once the requested spine node's output
            // has landed in `out`. Its upstream value sources precede it in
            // topological order, so they have already run.
            if tap == Some(*id) {
                break;
            }
        }
    }

    /// Seed the working buffer with the ornament-expanded source (shared with
    /// the rack, plan §4) — unless the active spine is headed by a pure
    /// generator (a stream node with no stream input, e.g. Euclidean), which
    /// defines its own stream and must not inherit the pattern's source notes
    /// (they would otherwise leak alongside the generated ones). The one seeding
    /// rule, shared by [`Self::expand_impl`] and [`Self::eval_prefix_at_tick`].
    fn seed_stream_input(
        &self,
        source: &[Note],
        tick: PatternTick,
        bpm: Bpm,
        gate: &dyn Fn(&Note) -> bool,
        note_scope: Option<&mut NoteScopeCtx<'_>>,
        out: &mut ExpansionBuffer,
    ) {
        if self.stream_head_is_generator() {
            out.clear();
        } else {
            seed_source_at_tick(source, tick, bpm, gate, note_scope, out);
        }
    }

    /// Node `prefix_len`'s **input stream at `tick`**: re-run spine nodes
    /// `[0, prefix_len)` into `out` (plan `note-grid.md` §11).
    /// `prefix_len == 0` (the node is the spine head) seeds the source only.
    /// `pool` carries the recursion budget for any timing node inside the prefix.
    /// `gate` is the caller's source-note gate (the engine's probability roll in
    /// playback, `|_| true` in freeze/preview) so an echo/retrigger fires iff its
    /// dry source note did — wet follows dry. One caveat: a `StreamOnset`
    /// envelope inside the prefix re-evaluates with its source-onset fallback
    /// (see `in_lookback` below), so a stream node modulated *by such an
    /// envelope* may decide differently in the probe than it did dry — the
    /// spec's documented approximation, not a purity break (still deterministic).
    /// Pure — a function of `(graph, source, tick, gate)`, independent of seek
    /// order.
    #[allow(clippy::too_many_arguments)]
    fn eval_prefix_at_tick(
        &self,
        prefix_len: usize,
        tick: PatternTick,
        source: &[Note],
        host: HostKey,
        bpm: Bpm,
        gate: &dyn Fn(&Note) -> bool,
        note_scope: Option<&mut NoteScopeCtx<'_>>,
        pool: &mut [ExpansionBuffer],
        out: &mut ExpansionBuffer,
    ) {
        match prefix_len
            .checked_sub(1)
            .and_then(|i| self.stream_spine.get(i))
        {
            // The node is the spine head: its input is the seeded source.
            None => self.seed_stream_input(source, tick, bpm, gate, note_scope, out),
            // Otherwise run the prefix, stopping after spine node `prefix_len - 1`
            // so `out` holds exactly this node's input. `in_lookback = true`: any
            // `StreamOnset` envelope inside the prefix uses its cheap source-onset
            // fallback, so this re-evaluation can never recurse back into a
            // stream backward-scan.
            Some(&tap) => {
                self.expand_impl(
                    source,
                    tick,
                    host,
                    bpm,
                    gate,
                    Some(tap),
                    note_scope,
                    pool,
                    true,
                    out,
                );
            }
        }
    }

    /// A `StreamOnset` [`NoteEnvelope`]'s level at `tick`: scan back for the most
    /// recent tick at which the *transformed terminal stream* had an onset, and
    /// return the envelope level that many ticks in. Bounded by the envelope
    /// window capped at [`MAX_ENV_STREAM_WINDOW`] (each probe re-runs the spine,
    /// so this scan gets a much tighter cap than the delay look-back) and stops
    /// at the first onset found; a fully silent window returns `0.0`. Runs each
    /// probe through [`Self::eval_prefix_at_tick`] with `in_lookback`, so it
    /// never recurses into another stream backward-scan. Non-recursive, bounded.
    ///
    /// Probes run with an **empty look-back pool**, so the envelope retriggers
    /// on the *dry* transformed onsets only — a delay's echoes do not retrigger
    /// it. This is deliberate RT protection, not a shortcut: with the pool
    /// threaded through, every probe re-runs each in-spine delay's own probes,
    /// and the measured worst case (silent window × 16-repeat delay) was a full
    /// audio-thread stall (~103% CPU vs ~12% dry-only, in-app 2026-07-12).
    #[allow(clippy::too_many_arguments)]
    fn envelope_stream_value(
        &self,
        env: &NoteEnvelope,
        tick: PatternTick,
        source: &[Note],
        host: HostKey,
        bpm: Bpm,
        gate: &dyn Fn(&Note) -> bool,
        mut note_scope: Option<&mut NoteScopeCtx<'_>>,
        pool: &mut [ExpansionBuffer],
    ) -> f32 {
        // The backward scan re-runs the whole spine per probe, so it is bounded by
        // a tighter window than the envelope's full length: a `StreamOnset`
        // envelope retriggers per transformed onset (close together), so a beat is
        // ample and keeps an idle-stream scan RT-safe.
        let window = env.window().min(MAX_ENV_STREAM_WINDOW);
        if window == 0 {
            return 0.0;
        }
        // No look-back budget (the caller opted out with `lookback: None`):
        // degrade to the source-onset value — the same fallback a prefix re-eval
        // uses (`in_lookback`), so every degraded path agrees.
        let Some((scratch, _)) = pool.split_first_mut() else {
            return env.value(tick, source);
        };
        let terminal = self.stream_spine.len();
        // Most recent onset first: scan back from `tick` and stop at the first
        // tick whose transformed output is non-empty. The empty pool makes each
        // probe dry-only (see the method doc — the anti-stall guarantee).
        for back in 0..=window {
            let Some(s) = tick.0.checked_sub(back) else {
                break;
            };
            self.eval_prefix_at_tick(
                terminal,
                PatternTick(s),
                source,
                host,
                bpm,
                gate,
                note_scope.as_deref_mut(),
                &mut [],
                scratch,
            );
            if !scratch.notes().is_empty() {
                return env.level_at(back);
            }
        }
        0.0
    }

    /// Apply a [`NoteDelay`] node at spine position `spine_pos`: keep the dry
    /// stream already in `out` and append decayed copies of the onsets that
    /// entered this node at ticks `tick − k·delay` (`k = 1..=repeats`). `pool` is
    /// consumed one buffer per look-back level; an empty `pool` (recursion cap
    /// reached, or the caller opted out) yields dry-only. Pure, RT-safe.
    #[allow(clippy::too_many_arguments)]
    fn apply_delay(
        &self,
        delay: &NoteDelay,
        spine_pos: usize,
        tick: PatternTick,
        source: &[Note],
        host: HostKey,
        bpm: Bpm,
        gate: &dyn Fn(&Note) -> bool,
        mut note_scope: Option<&mut NoteScopeCtx<'_>>,
        pool: &mut [ExpansionBuffer],
        out: &mut ExpansionBuffer,
    ) {
        // No spacing: nothing to probe, the dry signal is the whole output.
        let step = delay.delay_ticks.0;
        if step == 0 {
            return;
        }
        // No scratch left (a too-deep delay-behind-a-delay chain, or the caller
        // opted out of look-back): dry-only.
        let Some((scratch, rest)) = pool.split_first_mut() else {
            return;
        };
        // Clamp at use: `NormalizedValue` is `#[serde(transparent)]`, so a
        // hand-edited/MCP file can carry an out-of-range value — un-clamped, a
        // feedback > 1 would defeat the audibility floor (echoes that never
        // decay) and a negative one would kill audible echoes at k = 1.
        let feedback = delay.feedback.as_f32().clamp(0.0, 1.0);
        for k in 1..=u32::from(delay.live_repeats()) {
            let gain = feedback.powi(k as i32);
            // Stop once an echo rounds to silence — every later echo is
            // quieter still (spec §3).
            if gain < MIN_AUDIBLE_GAIN {
                break;
            }
            let back = k.saturating_mul(step);
            if back > MAX_NOTE_DELAY_TICKS {
                break; // past the look-back window
            }
            let Some(s) = tick.0.checked_sub(back) else {
                break; // before the pattern start — larger k only reaches further back
            };
            self.eval_prefix_at_tick(
                spine_pos,
                PatternTick(s),
                source,
                host,
                bpm,
                gate,
                note_scope.as_deref_mut(),
                rest,
                scratch,
            );
            for note in scratch.notes() {
                let mut echo = *note;
                echo.velocity = Velocity::new(note.velocity.as_f32() * gain);
                if !out.push(echo) {
                    return; // buffer full — nothing more fits this tick
                }
            }
        }
    }

    /// Apply a [`Ratchet`] node at spine position `spine_pos`: shorten the dry
    /// onsets in `out` to one subdivision and append the retriggers that land on
    /// `tick` from onsets `m·sub_ticks` earlier that are still sounding. Same
    /// bounded, pure look-back as [`Self::apply_delay`]; `pool` bounds recursion.
    #[allow(clippy::too_many_arguments)]
    fn apply_ratchet(
        &self,
        ratchet: &Ratchet,
        spine_pos: usize,
        tick: PatternTick,
        source: &[Note],
        host: HostKey,
        bpm: Bpm,
        gate: &dyn Fn(&Note) -> bool,
        mut note_scope: Option<&mut NoteScopeCtx<'_>>,
        pool: &mut [ExpansionBuffer],
        out: &mut ExpansionBuffer,
    ) {
        let sub = ratchet.sub_ticks.0;
        let count = ratchet.live_count();
        if sub == 0 || count <= 1 {
            return; // no subdivision — the dry note passes through untouched
        }
        // Subdivide the dry onset: replace a note longer than one subdivision with
        // a single subdivision, so it becomes the first of the stutter rather than
        // sustaining beneath it. (`None` = plays until cut ⇒ also shortened.)
        // Deliberately BEFORE the pool guard — shortening needs no look-back, so
        // even a pool-exhausted / opted-out ratchet keeps the node's dry contract
        // (a shortened onset) instead of flipping to an unshortened note.
        for note in out.notes_mut() {
            if note.duration.is_none_or(|d| d.0 > sub) {
                note.duration = Some(Duration(sub));
            }
        }
        // No scratch left: shortened dry onset only, no retriggers.
        let Some((scratch, rest)) = pool.split_first_mut() else {
            return;
        };
        // Clamp at use — same serde-transparent rationale as `apply_delay`.
        let decay = ratchet.decay.as_f32().clamp(0.0, 1.0);
        for m in 1..u32::from(count) {
            let gain = decay.powi(m as i32);
            if gain < MIN_AUDIBLE_GAIN {
                break; // rounds to silence
            }
            let back = m.saturating_mul(sub);
            if back > MAX_NOTE_DELAY_TICKS {
                break;
            }
            let Some(s) = tick.0.checked_sub(back) else {
                break;
            };
            self.eval_prefix_at_tick(
                spine_pos,
                PatternTick(s),
                source,
                host,
                bpm,
                gate,
                note_scope.as_deref_mut(),
                rest,
                scratch,
            );
            for note in scratch.notes() {
                // Retrigger only while the source note still sounds at `tick`,
                // and never past the note's end: the last retrigger of a note
                // that ends mid-subdivision is clipped to the remaining span.
                let remaining = match note.duration {
                    Some(d) => {
                        if back >= d.0 {
                            continue;
                        }
                        (d.0 - back).min(sub)
                    }
                    None => sub,
                };
                let mut hit = *note;
                hit.duration = Some(Duration(remaining));
                hit.velocity = Velocity::new(note.velocity.as_f32() * gain);
                if !out.push(hit) {
                    return; // buffer full
                }
            }
        }
    }

    /// Bake the graph over `[0, length)` into concrete `(tick, note)` pairs —
    /// the freeze / offline path. Non-RT (allocates; walks every tick, since
    /// generators may emit where no source note starts). Purity makes the bake
    /// identical to per-tick playback by construction (plan §7).
    #[must_use]
    /// Bake the graph over `[0, length)`, returning the baked notes and the total
    /// events **dropped** during the walk — a node hitting the 128-event
    /// [`ExpansionBuffer`] cap. Freeze surfaces this so an overflowing graph is
    /// reported rather than silently truncated (plan §7). [`Self::bake`] is the
    /// drop-discarding wrapper for callers that don't need the count.
    pub fn bake_counted(
        &self,
        source: &[Note],
        length: PatternTick,
        host: HostKey,
        bpm: Bpm,
        mut note_scope: Option<&mut NoteScopeCtx<'_>>,
    ) -> (Vec<(PatternTick, ExpandedNote)>, u32) {
        let mut work = ExpansionBuffer::new();
        // Non-RT path: allocate the timing look-back pool locally so echoes bake
        // exactly as they play (`freeze == playback`, plan §7).
        let mut pool = lookback_pool();
        let mut baked = Vec::new();
        let mut dropped: u32 = 0;
        for t in 0..length.0 {
            let tick = PatternTick(t);
            self.expand_at_tick(
                source,
                tick,
                host,
                bpm,
                |_| true,
                note_scope.as_deref_mut(),
                Some(&mut pool),
                &mut work,
            );
            // `work` is re-seeded (cleared → `dropped` reset) each tick, so this
            // sums the per-tick drops, the same accumulation the live engine does.
            dropped = dropped.saturating_add(work.dropped());
            for note in work.notes() {
                baked.push((tick, *note));
            }
        }
        (baked, dropped)
    }

    /// Bake the graph, discarding the drop count — see [`Self::bake_counted`].
    pub fn bake(
        &self,
        source: &[Note],
        length: PatternTick,
        host: HostKey,
        bpm: Bpm,
        note_scope: Option<&mut NoteScopeCtx<'_>>,
    ) -> Vec<(PatternTick, ExpandedNote)> {
        self.bake_counted(source, length, host, bpm, note_scope).0
    }

    /// The furthest past a source onset any stream node reaches — the strum tail
    /// of any strummed `Chord` node. The freeze walk extends by this so a strum
    /// near the pattern end bakes its full spread, mirroring
    /// [`crate::Pattern::freeze_processors`]. (Arp steps only fire while a source
    /// chord is held, so they need no extension.)
    #[must_use]
    pub fn max_strum_tail(&self) -> u32 {
        self.nodes
            .values()
            .filter_map(|c| match c {
                NoteModuleConfig::Processor(NoteProcessor::Chord(chord)) => {
                    Some(chord.strum_tail())
                }
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }

    /// The furthest past a source onset any **timing** node reaches — the delay
    /// tail of any [`NoteDelay`] node. The freeze walk extends by this (on top of
    /// [`Self::max_strum_tail`]) so echoes landing past the pattern end still
    /// bake, exactly the strum-tail idiom (plan `note-grid.md` §11).
    #[must_use]
    pub fn max_delay_tail(&self) -> u32 {
        self.nodes
            .values()
            .filter_map(|c| match c {
                NoteModuleConfig::NoteDelay(delay) => Some(delay.delay_tail()),
                NoteModuleConfig::Ratchet(ratchet) => Some(ratchet.ratchet_tail()),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    }

    /// The graph's whole forward reach past a source onset — every tail a freeze
    /// walk must cover (strum + timing echoes; the sum is a sound upper bound).
    /// The **one** method freeze walk-ends should consume, so a future tail
    /// contributor is added here instead of at every walk-end call site.
    ///
    /// A **generator-headed** spine gets no tail: the generator emits fresh
    /// onsets at *every* walked tick, so extending the walk would bake phantom
    /// material past the pattern end (inert at the current length, then
    /// surfacing as stale hits the moment the pattern is lengthened) rather
    /// than tails of in-range notes. Its echoes past the end are equally inert
    /// beyond the pattern length, so nothing playable is lost by stopping.
    #[must_use]
    pub fn max_walk_tail(&self) -> u32 {
        if self.stream_head_is_generator() {
            return 0;
        }
        self.max_strum_tail().saturating_add(self.max_delay_tail())
    }

    /// Whether the node `node_id` is a source-context node (arpeggiator or
    /// strummed chord) that **silently ignores** an upstream stream node. Both
    /// rebuild their stream from the raw source notes viewed through *pitch
    /// transforms only* (scale-quantize / block chord), so any other upstream
    /// spine node — a script, probability gate, generator, humanize, or another
    /// strummed chord — has no effect on them. Unlike the rack's locked chain
    /// order, the free canvas allows such wiring, so the GUI surfaces this as a
    /// warning. Returns `false` for non-source-context nodes.
    #[must_use]
    pub fn source_context_ignores_upstream(&self, node_id: NoteModuleId) -> bool {
        let Some(NoteModuleConfig::Processor(p)) = self.nodes.get(&node_id) else {
            return false;
        };
        let is_source_context = matches!(p, NoteProcessor::Arpeggiator(_))
            || matches!(p, NoteProcessor::Chord(c) if c.strum().0 > 0);
        if !is_source_context {
            return false;
        }
        self.stream_spine
            .iter()
            .take_while(|&&sid| sid != node_id)
            .any(|sid| !self.is_arp_pitch_transform(*sid))
    }

    /// Whether `node_id` is an upstream pitch transform an arp / strummed chord
    /// actually consumes (scale-quantize or a *block* chord).
    fn is_arp_pitch_transform(&self, node_id: NoteModuleId) -> bool {
        matches!(
            self.nodes.get(&node_id),
            Some(NoteModuleConfig::Processor(NoteProcessor::ScaleQuantize(_)))
        ) || matches!(
            self.nodes.get(&node_id),
            Some(NoteModuleConfig::Processor(NoteProcessor::Chord(c))) if c.strum().0 == 0
        )
    }
}

#[cfg(test)]
#[path = "note_graph/tests.rs"]
mod tests;
