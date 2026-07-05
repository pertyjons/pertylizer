# Plan: Modular Note & Pattern Graphs (Note Grid)

This plan describes the architecture, module specifications, and implementation
guidelines for introducing a modular **Note Grid** to Pertylizer: a
project-level pool of note-processing graphs (DAGs) that patterns, individual
notes, and (later) tracks reference — extending the linear
[NoteProcessor](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_processor.rs)
rack into a general system for generative sequencing, note transformation, and
routing.

---

## 1. Core Architecture

### 1.1. The purity contract carries over (CRITICAL)

The existing rack has a **locked contract** (documented on
[`NoteProcessor::process_at_tick`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_processor.rs#L1085)):
every processor is a **pure, deterministic function of `(config, source
material, tick)`** — step phase derived from `tick`, held chords derived from
the pattern, randomness via seeded hashes (`synth_core::hash::splitmix64`),
never mutable cross-tick state. The Note Grid keeps this contract for every
node. It is not an implementation detail; it is what the rest of the product
is built on:

- **Random access / seek**: `expand_at_tick` can be asked about any tick in any
  order. Stateful nodes would produce different music depending on where
  playback started.
- **Preview & probability gating**: the engine's gate callback
  ([`note_passes_probability`](file:///home/per/github/pertylizer/crates/synth_engine/src/sequencer_engine.rs#L199))
  and the tracker's preview (`|_| true`) re-expand the same ticks with
  different gates.
- **Freeze / bake**:
  [`Pattern::freeze_processors`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_processor.rs#L1647)
  walks every tick once and must reproduce exactly what playback would play.
- **Tracker per-stage columns**: `expand_at_tick_through` re-runs prefixes of
  the chain per rendered row.
- **Reproducible offline renders**: `analyze_section` and friends re-expand
  the same material outside the live engine.

Purity is also what makes the **shared graph pool** (§1.2) work at all: a pure
graph carries no runtime state, so any number of patterns, notes, and preview
columns can reference the *same* graph simultaneously with zero phase
conflicts or event leaks. There is **no `NoteGraphInstance`** — only static
config plus per-evaluation scratch buffers owned by the caller (the engine
already owns a `scratch_expansion: ExpansionBuffer` for exactly this reason).

Modules that *look* stateful are formulated as functions of tick, using idioms
that already exist in the rack:

| Apparent state              | Pure formulation (existing precedent)                                                                                                                                                  |
|-----------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| LFO phase                   | phase = f(tick) (tempo-synced), or f(tick, bpm) like `ArpRate`'s absolute-rate variants                                                                                                |
| Step-sequencer pointer      | `step = (tick / step_len) % steps` — same as the arpeggiator's step derivation                                                                                                         |
| Delay / echo / strum queues | **bounded look-back scan**: at tick `t`, scan source onsets in `[t − max_delay, t]` and emit the hits that land on `t` — exactly how per-note ornaments and strummed chords work today |
| Envelope level              | level = f(ticks since the most recent upstream onset ≤ t) — onset found by a bounded pattern scan                                                                                      |
| Round-robin index           | ordinal of the note among the pattern's onsets (bounded scan), hashed into an output index                                                                                             |
| Randomness                  | `splitmix64(seed ^ host_key ^ stable_note_key ^ tick)` — the `Humanize` / probability-roll idiom, widened with a host key (§1.2)                                                       |

### 1.2. The project-level graph pool

Graphs are **project assets, referenced by id** — the instrument model, not
the embedded-per-pattern model. One graph, many users; editing it updates
every user; the pool view gives the overview.

```rust
/// Static note-graph configuration stored in Song (serialized with the project).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NoteGraph {
    pub id: NoteGraphId,
    /// User-facing metadata, like instruments have: shown in the pool view.
    pub name: String,
    pub description: String,
    pub color: Option<Color32Def>,
    pub nodes: BTreeMap<NoteModuleId, NoteModuleConfig>,
    /// Sorted Vec, not HashSet — deterministic serialization (stable
    /// project-file diffs) and deterministic iteration.
    pub connections: Vec<NoteConnection>,
    /// Derived — NOT serialized. Recomputed on the UI thread on load and
    /// whenever connections change (Kahn's algorithm, same as
    /// ModuleGraph::calculate_processing_order). Cycles are rejected at
    /// connect time, so this never fails at play time.
    #[serde(skip)]
    pub processing_order: Vec<NoteModuleId>,
}
```

- The pool lives in
  **[Song](file:///home/per/github/pertylizer/crates/synth_sequencer/src/song.rs#L168)**
  (`note_graphs: Vec<NoteGraph>` + `next_note_graph_id`), serialized with the
  project like patterns and return busses.
- Hosts reference by id: `Pattern::note_graph: Option<NoteGraphId>`,
  `Note::note_graph: Option<NoteGraphId>`, later
  `SequencerTrack::note_graph` (§2).
- `NoteGraphId` / `NoteModuleId` are new newtypes in `synth_sequencer` (add to
  the newtype table in CLAUDE.md when implemented).
- Mutations go through `Song` methods and the existing `Arc<RwLock<Song>>`
  write path; the audio thread reads via the existing `try_read` snapshotting.

Sharing has consequences the instrument model already answers — adopt the
same answers:

- **Edit-affects-all**: editing a pooled graph changes every host. The editor
  header shows usage ("used by 3 patterns, 12 notes"), and a **"make
  unique"** action clones the graph and repoints the current host (the
  instrument-duplication move).
- **Seed decorrelation per host (automatic)**: every random draw mixes a
  stable **host key** (the referencing `PatternId` / `NoteId` / `TrackId`)
  into the hash. Without this, one "Humanize" graph shared by ten notes
  humanizes all ten *identically* — the same trap the `Humanize.seed` docs
  flag for per-placement reuse (NP0). Same graph → different but reproducible
  variation per host. A per-host seed override is a later refinement.
- **Deletion**: the GUI confirms with the usage list; deleting clears all
  references (hosts play dry). Independently, the runtime treats a dangling
  `NoteGraphId` as pass-through — never a panic, never silence (the YAMS
  dangling-`src` disable-and-keep philosophy).
- **Per-reference parameter overrides** ("same echo, but 1/8 on this
  pattern") are **deferred**; v1 answer is "make unique". Precedent when
  revisited: `PatternPlacement::length_override`.

### 1.3. Relation to the linear rack: one concept, two views

Long-term there is **one** processing concept — the graph — and the linear
rack is its degenerate (chain-shaped) case, not a parallel system:

- **Chain view first**: linear graphs get a rack-style list UI (the
  mainstream note-FX UX; Ableton-style). The full node editor is the
  power-user view of the *same* data. A graph with no splitters/Value cables
  renders naturally as a chain.
- Graph node configs **reuse the existing processor configs** (`ScaleQuantize`,
  `Chord`, `Arpeggiator`, `Humanize`) — the DSP stays single-source-of-truth.
- The shipped per-pattern rack (`Vec<NoteProcessor>`, locked chain order,
  NP7 MCP tools, tracker columns) **stays untouched through v1**; a pattern
  uses either the rack or a graph reference (`note_graph` set ⇒ rack
  ignored). A one-click "convert rack → graph" wraps the rack stages into a
  new pooled graph in chain order. Actually *migrating* racks to linear
  graphs (and retiring the rack code path) is a late phase, gated on the
  graph reaching feature parity (per-stage tracker columns included).

### 1.4. Real-Time Safety (RT-Safety Contract)

Graph evaluation happens inside the audio thread's per-tick expansion
(`SequencerEngine::collect_events_at_tick` → `expand_at_tick`). Note the rate:
at 960 PPQ ([`TICKS_PER_QUARTER`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/time.rs#L10))
and 120 BPM this is ~1920 evaluations/second **per active host** — the budget
conversation, not audio-block rate.

- **No allocation, no locks**: evaluation walks `processing_order` linearly
  over pre-allocated per-node buffers.
- **Topological sorting is UI-thread-only** (recomputed on edit/load, never on
  the audio thread — same rule as the audio `ModuleGraph`).
- **Pre-allocated scratch**: the engine owns one `NoteGridScratch` — an array
  of `MAX_NOTE_GRID_NODES` fixed-capacity note buffers (capacity
  `MAX_EXPANSION_EVENTS_PER_TICK = 128` each, the existing cap) plus per-node
  `Value`/`Gate` slots. Node count is hard-capped
  (`MAX_NOTE_GRID_NODES = 32`); adding a node past the cap is a UI/MCP-level
  error, never an audio-thread concern.
- **Overflow policy**: identical to
  [`ExpansionBuffer`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_processor.rs#L1312)
  — drop newest, count, report from the non-RT side. Never grow, never log on
  the audio thread.
- **Bounded look-back**: delay-flavored modules declare a maximum look-back
  window in ticks (`MAX_NOTE_DELAY_TICKS`, e.g. 2 bars at 4/4 = 7680); the
  per-tick source scan is bounded by source note count × window hits, the
  same cost shape as ornament expansion today.

---

## 2. Scopes: where a graph can be attached

A graph reference can sit at three levels. Evaluation chains **note → pattern
→ track** (innermost articulation first, broadest processing last), mirroring
how per-note ornaments already expand during source collection *before* the
pattern rack runs.

### 2.1. Note scope (`Note::note_graph`)

A graph attached to a single note — the generalization of the per-note
`Ornament`: "this note is strummed / rolled / arpeggiated / echoed". Runs
during source collection on exactly that note's material (ornament-expansion
precedent, including lead-in emission before the onset tick). Cheap, pure,
and the natural first consumer of the pool: many notes pointing at one
"flam" graph, decorrelated by host key. Long-term, `Ornament` presets can be
expressed as factory note-scope graphs.

### 2.2. Pattern scope (`Pattern::note_graph`)

The v1 core: the graph processes the pattern's expanded source stream —
everything §1 and §4–5 describe.

### 2.3. Track scope (`SequencerTrack::note_graph`) — later phase

The graph processes the **merged stream of all placements on the track**,
applied in `collect_events_at_tick` after pattern expansion. This scope earns
its extra engine work with two things the pattern scope *cannot* do:

- **Tails across pattern boundaries**: a per-pattern echo dies when its
  placement ends; a track-scope echo keeps emitting past it.
- **A future live-input path**: track-level note FX is where live keyboard
  input could eventually route through a graph (the Ableton/Bitwig model) —
  out of scope here, but this is the seam it would use.

Costs to solve when this phase starts: the pure contract's "source material"
becomes an arrangement slice (cross-placement look-back), and freeze semantics
need an answer (bake to what?). **Mixer/return buses are explicitly not a
scope** — no note events flow there; note graphs belong to sequencer tracks
feeding instruments.

**Known limitation (state it in docs, don't discover it late):** graphs run
at pattern/arrangement expansion, so until a live-input path exists, notes
played live on the keyboard bypass them. This is the deliberate trade for the
determinism in §1.1 — Pertylizer's Note Grid is a *deterministic pattern
transformer*, not a live stateful modular (Bitwig-style random walks /
Turing machines are out of model).

---

## 3. Data Flows and Port Types

Three signal types flow through Note Grid connections. **All are evaluated at
tick rate** during expansion (not at audio-block rate — the Note Grid never
touches audio buffers):

1. **`NoteStream`** — the per-tick set of expanded notes (semantically a
   `&[ExpandedNote]` slice for the current tick). Empty on most ticks.
2. **`Value`** — a continuous control scalar, recomputed per tick. Normalized
   (`0.0..1.0`) or bipolar (`-1.0..1.0`), consistent with the YAMS scaling
   contract. Consumers that map a `Value` onto note fields sample it at the
   note's onset tick (no zipper concern — notes are events, not audio).
3. **`Gate`** — a trigger that is high on specific ticks (clocking step
   sequencers, triggering envelopes). Derived from tick arithmetic or from
   `NoteStream` onsets.

Connections are type-checked at connect time (UI/MCP), like the patch editor's
port types.

---

## 4. Anatomy of a NoteStream Event

A `NoteStream` event is the existing
[`ExpandedNote`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_processor.rs#L1279):
pitch, velocity, duration, legato, glide, expression. Two model facts shape
every module design:

- **There is no `start` field.** Expansion is pull-based: everything a node
  emits at tick `t` *sounds at* `t`. A module cannot "move a note later" by
  editing a timestamp — it must emit nothing at the onset tick and emit the
  note when the later tick is scanned (the look-back idiom, §1.1). This is how
  ornaments, strum, and the arp already work.
- **Pitch is integer MIDI**
  ([`Pitch(u8)`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/pitch.rs#L22),
  0–127). There is no per-note detune on `Note`/`ExpandedNote` today; see
  `MicrotonalTuner` in §5 for the extension this implies.

Fields modules can read/write:

* **Core**: `pitch` (MIDI 0–127; transpose, quantize), `velocity` (0.0–1.0;
  scale, humanize), `duration` (ticks, `None` = plays until cut; lengthen /
  shorten).
* **Expression**
  ([`NoteExpression`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note.rs#L124)):
  `vibrato` (depth/rate/delay/**shape**), `accent` (velocity ×), `gate`
  (duration fraction, staccato–tenuto), `ghost` (forced low velocity),
  `probability` (0.0–1.0 — note: resolved by the **engine's** seeded roll at
  emission, per-occurrence via `roll_nonce`; a graph node that wants its own
  probability behavior rolls its own seeded hash instead of writing this
  field).
* **Articulation**: `legato` (tie to successor), `glide` (per-note
  portamento/glissando: from/time/interp).
* **Not carried through the graph**: `ornament` — ornaments are expanded
  *during source collection* (before the rack/graph runs) so upstream quantize
  snaps their grace pitches; graph nodes see the resulting hits, not the
  config.

**Division of planes (design invariant):** `NoteExpression` is the **data
plane** — per-note facts the composer sets on a specific note, edited in the
piano roll / tracker sub-columns. The Note Grid is the **transform plane** —
rules over streams of notes, which *read and write those same fields*.
`ExpandedNote`'s expression block is the payload format graph modules operate
on, so the grid depends on `NoteExpression` rather than replacing it. Neither
plane is phased out by the other.

---

## 5. Module Types

Every module below is specified pure (§1.1). Annotations mark the ones whose
pure formulation or engine wiring needs extra care.

### A. Rhythmic & Timing Generators
* **`EuclideanGenerator`**: emits trigger notes on Euclidean onsets —
  `hit = f(tick / step_len % steps, pulses, rotation)`, pure tick arithmetic.
  Pitch/velocity from config or from a `Value` input.
* **`NoteDelay / Echo`**: decaying repeats at tick offsets. Look-back scan
  (§1.1); repeats capped (`max_repeats × sources` bounded by the 128-event
  buffer cap). No queue.
* **`Strummer`**: micro-delays chord tones — already implemented as
  `Chord::expand_strummed`; the graph node reuses that code path.
* **`Ratchet`**: subdivides a note into fast repeats across its duration —
  same per-tick window scan as ornament rolls (`expand_ornament_at`
  precedent).

### B. Harmonic & Pitch Modifiers
* **`ChordGenerator`**: wraps the existing `Chord` config
  (`MAX_CHORD_INTERVALS = 8`). Must also participate in the **held-pitch
  seam**
  ([`expand_pitch`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_processor.rs#L1072))
  so a downstream arpeggiator sees its tones — in graph terms: the arp node
  resolves held notes through its *upstream path*, the DAG generalization of
  the rack's `upstream: &[NoteProcessor]` slice.
* **`ScaleQuantizer`**: wraps existing `ScaleQuantize` (root + `ScaleMask`).
* **`PitchRandomizer / Drift`**: seeded-hash pitch/octave offsets (Humanize
  idiom); optional in-scale constraint by chaining a quantizer after it.
* **`MicrotonalTuner`** *(deferred to a late phase)*: requires a per-note
  detune field (`Option<Cents>`) added to `ExpandedNote` +
  `SequencerEvent::NoteOn` + voice wiring. The engine-side landing path exists
  (continuous voice pitch, `set_voice_pitch`), but the sequencer-side field
  and event plumbing are new work — do not block v1 on it.

### C. Modulators (Value/Gate sources)
* **`NoteLfo`**: tempo-synced `Value` LFO — phase derived from tick. An
  absolute-Hz rate variant takes `bpm` (already threaded into expansion for
  the arp's absolute rates).
* **`NoteEnvelope`**: AD/ADSR-shaped `Value`, level = f(ticks since the most
  recent onset on its `Gate`/`NoteStream` input).
* **`StepLfo`**: step-sequenced `Value`; pointer = tick arithmetic.

### D. Utility & Routing
* **`KeyZoneSplitter`** / **`VelocitySplitter`**: route notes to output A/B by
  pitch/velocity range. Pure filters.
* **`Alternate / RoundRobin`**: routes the *k*-th onset to output `k % n`;
  ordinal derived by a bounded scan of upstream onsets (§1.1).
* **`ProbabilityGate`**: seeded-hash pass/block per note occurrence
  (`splitmix64(seed ^ host_key ^ note_key ^ tick)`), reproducible like
  Humanize. Optional `Value` input modulating the threshold.

**Routing scope (v1)**: the graph has exactly one terminal `NoteStream`
output, feeding the host's instrument — same destination contract as the
rack. Routing different splitter branches to *different instruments/tracks*
crosses the pattern→track binding and is explicitly **out of scope for v1**
(splitters are for building different processing per branch, then merging).

**Merging**: a `NoteStream` input port accepts multiple connections; streams
concatenate in deterministic (connection-sorted) order, subject to the buffer
cap.

---

## 6. Scripting: NoteScript Modules (YAMS dialect)

**Reality check**: YAMS today
([yams.md](file:///home/per/github/pertylizer/docs/yams.md)) is an expression
language — `src`/`let`/`state` plus exactly one `out =` (the audio dialect
adds `out.left/right`). It has **no event callbacks, no `on note(...)`
blocks, no `emit()` side effects**. NoteScript is therefore a new *dialect*
gated by `CompileOptions` (the precedent is `CompileOptions::audio_rate`,
which unlocks `in`/`first_sample`/`out.left`), not free functionality. Split
into two modules so the cheap case stays cheap:

### 6.1. `NoteScriptTransform` (dialect: `note_event`) — v1

Runs once per note event, 1-to-1, stateless. New grammar unlocked by the
dialect (everything stays `f32`, NaN-free, same VM):

- **Reads**: `note_pitch` (raw MIDI 0–127, matching `mtof`'s convention),
  `note_vel` (0..1), `note_dur` (ticks), `tick`, plus `in1..in4` (`Value`
  ports).
- **Writes** (statements, like the audio dialect's `out.left`): `out_pitch`,
  `out_vel`, `out_dur`, `out_gate`, `out_prob`. Unwritten fields pass
  through. `out_vel = -1` (any negative) drops the note.

```yams
# Transpose an octave; LFO from in1 shapes velocity.
out_pitch = note_pitch + 12
out_vel = note_vel * (0.5 + 0.5 * in1)
```

Because the VM is deterministic and per-event state resets, this module is
pure by construction. Compile on the UI/MCP thread, run bytecode on the audio
thread — the existing YAMS execution model.

### 6.2. `NoteScriptGenerator` (dialect: `note_event` + `emit`) — later phase

Adds an `emit(pitch, vel, dur)` / `emit(pitch, vel, dur, delay_ticks)` builtin
(callable multiple times per evaluation, hard-capped at
`MAX_SCRIPT_EMITS = 16`). **Purity is preserved by the look-back idiom**: the
script runs deterministically for a source note; at tick `t` the engine
re-evaluates it for each source onset `s` in the look-back window and keeps
only emissions with `s + delay == t`. `delay_ticks` is clamped to
`MAX_NOTE_DELAY_TICKS`. No queues, no cross-tick VM state.

```yams
# Strummed major triad, 30 ticks apart.
emit(note_pitch, note_vel, note_dur)
emit(note_pitch + 4, note_vel, note_dur, 30)
emit(note_pitch + 7, note_vel, note_dur, 60)
```

This is a real language-surface change (statement-only programs, a
side-effect builtin, no `out`): budget parser/compiler/VM work, `yamsfmt`
support, and doc updates in `docs/yams.md` as its own phase.

---

## 7. Integration Surfaces (each needs an answer, not all in v1)

- **Freeze / bake**: graph version of `freeze_processors` — walk every tick,
  bake to plain notes, clear the host's `note_graph` reference (the pooled
  graph itself survives; other hosts may use it). Purity makes the bake
  identical to playback by construction. The look-back window extends the
  walk range the same way `strum_tail` does today.
- **Tracker**: the rack's per-processor columns (`expand_at_tick_through`)
  don't generalize to a DAG. v1: tracker shows a single "graph output" column
  (like the existing read-only NP output column); per-node taps are a later
  GUI feature (pick a node, render its output buffer).
- **MCP**: pool CRUD + reference setters —
  `list_note_graphs` / `create_note_graph` / `delete_note_graph` (returns the
  usage list), `add_note_graph_module`, `connect_note_graph`,
  `set_note_graph_module_param`, `get_note_graph`, and
  `set_pattern_note_graph` / `set_note_note_graph` (array-taking variants per
  the MCP array-tools convention; descriptor-driven validation listing valid
  ports on error).
- **Offline analyzers**: `analyze_section` et al. must expand through the
  graph exactly like playback (they share `expand_at_tick`, so this falls out
  — add a regression test).
- **Undo**: graph edits go through the same undo path as pattern edits; note
  that undoing an edit to a shared graph affects all hosts, same as
  instrument-patch undo.
- **Dropped-event reporting**: surface per-node drop counters (buffer cap,
  emit cap, look-back clamp) as UI warnings, read from the non-RT side.

---

## 8. UI/UX Integration (egui)

- **Graph pool view**: a project-level list like the instrument view — name,
  color, description, usage count ("3 patterns, 12 notes"), create /
  duplicate / delete, click-through to the editor. This is the overview the
  pool model buys.
- **Editor, two tiers**: **chain view first** (rack-style ordered list for
  linear graphs — the mainstream UX), node view for graphs with routing.
  The node view builds on the **`egui::Scene`-based patch editor**
  (`plans/patch-editor-rewrite-plan.md`) — do not extend the legacy
  manual-coordinate canvas that rewrite is retiring. Reuse `ModuleNode`, port
  and cable rendering, with the Note Grid's own port-type palette.
- **Host binding UI**: pattern editor gets a graph selector (like the track's
  instrument selector); the piano roll / tracker note inspector gets the same
  selector per note.
- **Visual feedback**: cables pulse when events pass. Requires audio→UI
  telemetry (per-edge event counters over the existing `EngineEvent` ring,
  throttled). Optional polish phase — not v1.

---

## 9. Phasing

1. **Data model + evaluation core** (`synth_sequencer`): `NoteGraph` pool in
   `Song`, ids, topo order, `NoteGridScratch`, pure per-tick evaluation with
   host-key seed mixing; nodes wrapping the four existing processors; unit
   tests incl. seek-order-independence, bake-equals-playback, and
   shared-graph decorrelation.
2. **Engine + persistence** (pattern scope): `Pattern::note_graph` binding,
   expansion path, save/load, dangling-reference pass-through, freeze,
   drop-counter plumbing.
3. **Module catalog**: splitters, round-robin, probability gate, Euclidean,
   delay/echo, ratchet, LFO/envelope/step-LFO `Value` sources.
4. **MCP surface** + analyzer regression tests.
5. **GUI**: pool view + chain view + host selectors; tracker output column.
6. **NoteScriptTransform** (YAMS `note_event` dialect).
7. **Note scope**: `Note::note_graph`, collection-time evaluation (ornament
   seam), inspector UI.
8. **Node view** on the Scene-based editor (routing graphs become editable).
9. **Track scope**: merged-stream evaluation, cross-placement look-back,
   freeze story.
10. **Later**: NoteScriptGenerator (`emit`), MicrotonalTuner (per-note detune
    field + event plumbing), per-reference overrides, rack→graph migration
    (retire the rack code path at feature parity), cross-track routing, cable
    telemetry, per-node tracker taps.
