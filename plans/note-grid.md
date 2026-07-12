# Plan: Modular Note & Pattern Graphs (Note Grid)

This plan describes the architecture, module specifications, and implementation
guidelines for introducing a modular **Note Grid** to Pertylizer: a
project-level pool of note-processing graphs that patterns, individual notes,
and (later) tracks reference — extending the linear
[NoteProcessor](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_processor.rs)
rack into a general system for generative sequencing, note transformation, and
routing.

---

## 0. Design decisions (v1 scope) — read this first

This plan was scoped down from a full DAG to a **linear-first** design. The
decisions below are settled; the rest of the document elaborates them. Anything
marked *earned escalation* is deliberately deferred until real usage proves it
is needed — the same discipline applied to the audio-graph work.

1. **Linear-first, DAG deferred.** The **NoteStream spine is constrained
   linear** in v1 (one chain, no splitters/merge). The data model is already
   graph-shaped (`nodes` + `connections`), so the DAG is not a different system
   — it is *relaxing the linear-stream validation* (§1.3). Splitters, merges,
   and multi-input `NoteStream` ports are **earned escalation** (§5, §9).
2. **Value/Gate modulation ships in v1.** The one genuinely non-serial thing a
   linear note chain needs is a control side-input (an LFO shaping velocity, an
   envelope shaping gate). These are `Value`/`Gate` connections from a source
   node to a downstream node's parameter — they do **not** branch the stream,
   so they are allowed under the linear-stream constraint (§3).
3. **One graph per scope.** A pattern references **one** graph
   (`Option<NoteGraphId>`), not an ordered list. Chaining behaviors is done by
   ordering **nodes inside the one graph**, not by stacking graphs. A per-scope
   `Vec` of graphs is *earned escalation* (§1.2). Cross-scope composition is a
   fixed pipeline (§2), not a collision.
4. **The pooled graph is a project asset, like an instrument.** Referenced by
   id, shared by many hosts, edited in one place, decorrelated per host by a
   seed key (§1.2).
5. **The rack will be retired.** The shipped per-pattern `Vec<NoteProcessor>`
   rack stays untouched through v1, but there **is a path to its removal**: the
   four processor algorithms migrate into graph nodes, and the rack container
   dies once the graph reaches feature parity (§1.3, §9).
6. **The Note Grid view mirrors the instrument (Rack) view.** New `AppView`
   tab: topbar + left pool list + center **egui::Scene node canvas** reusing the
   shipped patch editor. The pattern editor gets only a **dropdown** to bind one
   graph. This inverts the old "chain-list first, node view later" ordering —
   the Scene node view is v1, the compact rack-list is an optional later
   alternate (§8).
7. **The pure per-tick evaluation core is unchanged** and is the first thing
   built and gated (§1.1, §9 phase 1). It is identical for linear and DAG — the
   topology sits on top of it.

### 0.8. Pre-implementation sub-specs (from technical review — verified against code)

A technical review checked the plan against the actual seams. Every finding was
verified against the code and holds. Four are **hard sub-specs that must be
written before code**, because they change a type or a semantic:

1. **`NoteEventKey` (runtime-only origin key).** The randomness formula assumes a
   `stable_note_key`, but
   [`ExpandedNote`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_processor.rs#L1279)
   carries no source-note id or origin tick — its start tick is *implicit*. And
   the engine's own probability roll seeds on the **source `Note::id`**
   ([`note_passes_probability`](file:///home/per/github/pertylizer/crates/synth_engine/src/sequencer_engine.rs#L190)),
   which generated notes never have. So a graph node's seeded hash needs a
   runtime-only key carried on the expanded event — `NoteEventKey` = origin note
   id + origin tick + emit ordinal — not a field that exists today. Until it
   lands, the documented fallback is `tick ⊕ pitch ⊕ buffer_index`:
   deterministic but weaker under reorder/merge. Each random-using module must
   state which key it uses. (Touches §1.1, §1.2, §5.)
2. **`eval_prefix_at_tick` — bounded look-back through transforms.** "Scan source
   onsets in `[t − max_delay, t]`" is only correct when the timing node sits
   *directly* on source. In a chain `Quantize → Delay → Humanize`, the delay must
   see the *upstream-transformed* onsets at earlier ticks, which means
   re-evaluating the upstream prefix at each of those ticks — a bounded
   `eval_prefix_at_tick(prefix_len, tick, scratch_slot)` with **hard recursion
   and repeat caps** so a delay-behind-a-delay cannot explode. The rack today only
   ever evaluates the current tick
   ([`expand_at_tick_through`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_processor.rs#L1607)),
   so this is genuinely new engine cost. **SHIPPED in phase 11** (bounded prefix
   re-eval, recursion budget as a consumable scratch slice, termination + cost
   bound); the pre-implementation `note-grid-timing-lookback.md` spec is now in
   code and was removed. (Touches §1.4, §5.A, §9.)
3. **serde / schema migration rules.** Adding `Song::note_graphs` and
   `Pattern::note_graph` changes `project.schema.json`, which a drift test
   byte-compares against regenerated output
   ([`schemas_validate_examples.rs`](file:///home/per/github/pertylizer/crates/pertylizer/tests/schemas_validate_examples.rs)).
   New fields take `#[serde(default, skip_serializing_if = …)]` (the existing
   `Song` / `Pattern` convention) so old projects load, and `gen_schemas` must be
   re-run and committed. This is why the pool is wired into `Song` only at phase 2
   — the phase-1 spike stays on standalone types with no serde/schema surface
   (§9). (Touches §1.2, §7, §9.)
4. **Value / Gate cycle & ordering rules.** Cycle validation must include
   `Value` / `Gate` edges, not just the `NoteStream` spine, or a modulation loop
   is invisible to the linear-chain check. Rule (v1): a `Value` / `Gate` edge may
   run only from an independent/upstream node to a downstream node's param — **no
   cycles, and latency edges disallowed**. A one-tick-latency feedback edge would
   need a previous-tick value, i.e. cross-tick state (breaks purity) or its own
   pure look-back rule (§0.8.2); defer it. The audio graph already rejects cycles at
   connect time via a Kahn sort
   ([`synth_engine.rs`](file:///home/per/github/pertylizer/crates/synth_engine/src/synth_engine.rs#L522));
   extend the same check to the note graph. (Touches §3.)

Smaller verified corrections are folded inline: `Color32Def` → `TrackColor`
(§1.2); `processing_order` rebuilt on **every** load/mutation path, not just the
UI thread (§1.2); RT wording scoped to the evaluator (§1.4); probability is not a
writable field and `NoteScriptTransform` drops `out_prob` + clamps its outputs
(§6.1); freeze lives on `Song` (§7); and the patch-editor reuse planned as an
adapter/refactor, since `PatchEditor` is coupled to the audio `ModuleType` /
instrument (§8).

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
| Randomness                  | `splitmix64(seed ^ host_key ^ stable_note_key ^ tick)` — the `Humanize` / probability-roll idiom, widened with a host key (§1.2); `stable_note_key` needs a new runtime `NoteEventKey` (§0.8.1)                                                       |

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
    /// Sequencer-side color — `synth_sequencer` has NO egui dependency, so this
    /// is the crate's own `TrackColor` (RGB + `from_hex`), NOT egui's `Color32`.
    pub color: Option<TrackColor>,
    pub nodes: BTreeMap<NoteModuleId, NoteModuleConfig>,
    /// Sorted Vec, not HashSet — deterministic serialization (stable
    /// project-file diffs) and deterministic iteration.
    ///
    /// v1: the `NoteStream` axis is validated LINEAR (one chain, no
    /// splitters/merge); `Value`/`Gate` modulation connections (source → a
    /// downstream param) are allowed because they do not branch the stream.
    /// The DAG escalation (§5) is relaxing that validation — no data migration.
    pub connections: Vec<NoteConnection>,
    /// Derived — NOT serialized. Rebuilt by `NoteGraph::rebuild_derived()`
    /// (Kahn's algorithm, same as ModuleGraph::calculate_processing_order) on
    /// **every** path that loads or mutates the graph — UI, MCP, and headless —
    /// before the `Song` write lock is released. Never recomputed on the audio
    /// thread, which must never observe a dirty or cyclic order. Cycles
    /// (including `Value`/`Gate` edges, §0.8.4) are rejected at connect time, so
    /// this never fails at play time. Trivial in the linear case, already general
    /// enough for the DAG.
    #[serde(skip)]
    pub processing_order: Vec<NoteModuleId>,
}
```

- The pool lives in
  **[Song](file:///home/per/github/pertylizer/crates/synth_sequencer/src/song.rs#L168)**
  (`note_graphs: Vec<NoteGraph>` + `next_note_graph_id`), serialized with the
  project like patterns and return busses. **Wired into `Song` at phase 2** — the
  phase-1 spike keeps `NoteGraph` standalone so it carries no serde/schema
  surface (§0.8.3, §9).
- Hosts reference by id, **one graph per scope**: `Pattern::note_graph:
  Option<NoteGraphId>`, `Note::note_graph: Option<NoteGraphId>`, later
  `SequencerTrack::note_graph` (§2). An ordered `Vec` of graphs per scope is
  *earned escalation* — chaining is done inside the one graph (decision §0.3).
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

### 1.3. Linear-first, and the path to retiring the rack

Long-term there is **one** processing concept — the graph — and the linear
rack is its degenerate (single-chain) case, not a parallel system. v1 realizes
this by *constraining* the graph, not by building a lesser one:

- **The NoteStream spine is linear in v1.** Connect-time validation (UI/MCP)
  rejects any topology that branches or merges the `NoteStream` axis: each
  node has one stream input and one stream output, terminating at the graph's
  single output. This keeps evaluation trivially ordered and keeps the
  held-pitch seam (§5.B) unambiguous — the arpeggiator's upstream is the single
  chain before it, exactly like the rack today.
- **The DAG is earned escalation, not a rewrite.** Splitters, merges, and
  multi-input stream ports are unlocked by *relaxing* that validation. The data
  model (`nodes` + `connections` + `processing_order`) is already general; only
  the merge/branch *evaluation semantics* and the connect-time rule are new
  work (§5, §9). No serialization change, no re-plumbing.
- Graph node configs **reuse the existing processor configs** (`ScaleQuantize`,
  `Chord`, `Arpeggiator`, `Humanize`) — the DSP stays single-source-of-truth.

**The rack will be retired — there is a path (decision §0.5).** The shipped
per-pattern rack (`Vec<NoteProcessor>`, locked chain order, NP7 MCP tools,
tracker columns) **stays untouched through v1**; a pattern uses either the rack
or a graph reference (`note_graph` set ⇒ rack ignored). A one-click "convert
rack → graph" wraps the rack stages into a new pooled graph in chain order.
Retirement is a **late phase** (§9), gated on the graph reaching feature parity
(per-stage tracker taps included), and it is largely an **extraction**: the
four algorithms + their config structs currently live coupled to the rack in
[`note_processor.rs`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_processor.rs);
retiring the rack means extracting the algorithm functions so a graph node is
their home, then deleting the `Vec<NoteProcessor>` container, `process_at_tick`
orchestration, NP7 tools, and tracker per-stage columns. Mechanical, not a
rewrite. The algorithm survives; the container dies.

### 1.4. Real-Time Safety (RT-Safety Contract)

Graph evaluation happens inside the audio thread's per-tick expansion
(`SequencerEngine::collect_events_at_tick` → `expand_at_tick`). Note the rate:
at 960 PPQ ([`TICKS_PER_QUARTER`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/time.rs#L10))
and 120 BPM this is ~1920 evaluations/second **per active host** — the budget
conversation, not audio-block rate.

- **No allocation *in the graph evaluator*, no locks**: after scratch init the
  evaluator walks `processing_order` linearly over pre-allocated per-node
  buffers. This is scoped to the evaluator, not the whole engine path — the
  surrounding engine legitimately owns `Vec<SequencerEvent>` output and
  scratch `Vec`s; the note buffers, like
  [`ExpansionBuffer`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_processor.rs#L1312),
  allocate their full capacity once at construction and never grow.
- **Topological sorting is non-RT-side only** (rebuilt via
  `NoteGraph::rebuild_derived()` on the UI / MCP / headless side on edit/load,
  §1.2 — never on the audio thread, same rule as the audio `ModuleGraph`).
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
  window in ticks (`MAX_NOTE_DELAY_TICKS`, e.g. 2 bars at 4/4 = 7680). When such
  a node reads *source directly*, the per-tick scan is bounded by source note
  count × window hits — the same cost shape as ornament expansion today. When it
  reads *downstream of a transform*, seeing earlier-tick upstream output requires
  the bounded `eval_prefix_at_tick` model with hard recursion/repeat caps
  (§0.8.2) — an unsolved spec, so timing nodes are constrained/deferred until it
  exists.

---

## 2. Scopes: where a graph can be attached

A graph reference can sit at three levels, **one graph per level** (§0.3).
Evaluation chains **note → pattern → track** (innermost articulation first,
broadest processing last), mirroring how per-note ornaments already expand
during source collection *before* the pattern rack runs. This fixed pipeline is
the answer to "won't the scopes collide?": they do not — each stage's output is
the next stage's input, so they **consolidate by chaining**, never fight.

```text
   source notes (pattern data)
      │
      ▼  ① NOTE-scope graph     ← per note: articulation (flam / strum / arp of ONE note),
      │     expands during source collection — same seam as ornaments today
      ▼  ② PATTERN-scope graph  ← processes the pattern's resulting stream (v1 core)
      │
      ▼  ③ TRACK-scope graph    ← processes the track's merged stream (later phase)
      │
      ▼  instrument
```

- **Same graph on note and pattern is allowed** — it simply runs at two stages
  (① and ②), decorrelated by host key (NoteId vs PatternId) so even identical
  configs draw different randomness. In practice the two scopes carry different
  roles: note = articulation, pattern = harmony/rhythm processing.

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
   `&[ExpandedNote]` slice for the current tick). Empty on most ticks. **In v1
   the stream axis is linear** (§1.3): one chain, no splitter/merge.
2. **`Value`** — a continuous control scalar, recomputed per tick. Normalized
   (`0.0..1.0`) or bipolar (`-1.0..1.0`), consistent with the YAMS scaling
   contract. Consumers that map a `Value` onto note fields sample it at the
   note's onset tick (no zipper concern — notes are events, not audio).
3. **`Gate`** — a trigger that is high on specific ticks (clocking step
   sequencers, triggering envelopes). Derived from tick arithmetic or from
   `NoteStream` onsets.

**`Value`/`Gate` modulation is the one non-serial element v1 needs, and it
ships in v1 (decision §0.2).** A modulation source node (`NoteLfo`,
`NoteEnvelope`, `StepLfo`) produces a `Value`/`Gate` that connects to a
**downstream node's parameter** (mod-matrix style — the app already has this
idiom in the dynamic Mod Matrix). This is a side-input: it feeds a param, it
does **not** branch the `NoteStream`, so it is permitted under the linear-stream
constraint. On the Scene canvas (§8) it renders as a cable from the source node
to the target node's param.

Connections are type-checked at connect time (UI/MCP), like the patch editor's
port types. **Cycle validation must span all edge types, not just the
`NoteStream` spine** (§0.8.4): a `Value`/`Gate` feedback loop is invisible to a
linear-chain check, so the Kahn sort that rejects cycles at connect time must
include `Value`/`Gate` edges. Rule (v1): a `Value`/`Gate` edge runs only from an
independent/upstream node to a downstream node's param — **no cycles, and latency
edges are disallowed in v1** (a one-tick-latency edge needs a previous-tick value:
cross-tick state, which breaks purity, or its own pure look-back rule §0.8.2 —
deferred).

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
pure formulation or engine wiring needs extra care, and which ones are **earned
escalation** (built only when the DAG lands).

### A. Rhythmic & Timing Generators
* **`EuclideanGenerator`**: emits trigger notes on Euclidean onsets —
  `hit = f(tick / step_len % steps, pulses, rotation)`, pure tick arithmetic.
  Pitch/velocity from config or from a `Value` input. Source-independent, so
  linear-safe and unaffected by the look-back spec.
* **`NoteDelay / Echo`** *(gated on §0.8.2)*: decaying repeats at tick offsets.
  Look-back scan; repeats capped (`max_repeats × sources` bounded by the
  128-event buffer cap). No queue. **Constrained to read source directly** (or be
  the graph's first node) until `eval_prefix_at_tick` is specified — a delay
  downstream of a transform needs earlier-tick upstream re-evaluation, which does
  not exist yet.
* **`Strummer`**: micro-delays chord tones — already implemented as
  `Chord::expand_strummed`; the graph node reuses that code path.
* **`Ratchet`** *(gated on §0.8.2)*: subdivides a note into fast repeats across
  its duration — same per-tick window scan as ornament rolls
  (`expand_ornament_at` precedent). Same source-only constraint as Delay/Echo
  until the look-back spec lands.

### B. Harmonic & Pitch Modifiers
* **`ChordGenerator`**: wraps the existing `Chord` config
  (`MAX_CHORD_INTERVALS = 8`). Must also participate in the **held-pitch
  seam**
  ([`expand_pitch`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_processor.rs#L1072))
  so a downstream arpeggiator sees its tones. **In v1 the stream is linear**, so
  the arp node resolves held notes through the single upstream chain — exactly
  the rack's `upstream: &[NoteProcessor]` slice. Resolving held pitch through a
  *branched* upstream is part of the DAG escalation.
* **`ScaleQuantizer`**: wraps existing `ScaleQuantize` (root + `ScaleMask`).
* **`PitchRandomizer / Drift`**: seeded-hash pitch/octave offsets (Humanize
  idiom); optional in-scale constraint by chaining a quantizer after it.
* **`MicrotonalTuner`** *(deferred to a late phase)*: requires a per-note
  detune field (`Option<Cents>`) added to `ExpandedNote` +
  `SequencerEvent::NoteOn` + voice wiring. The engine-side landing path exists
  (continuous voice pitch, `set_voice_pitch`), but the sequencer-side field
  and event plumbing are new work — do not block v1 on it.

### C. Modulators (Value/Gate sources) — ship in v1 (§0.2)
* **`NoteLfo`**: tempo-synced `Value` LFO — phase derived from tick. An
  absolute-Hz rate variant takes `bpm` (already threaded into expansion for
  the arp's absolute rates).
* **`NoteEnvelope`**: AD/ADSR-shaped `Value`, level = f(ticks since the most
  recent onset). **v1 constraint (§0.8.2):** the onset source must be an explicit
  `Gate` (tick-arithmetic or source-derived, RT-cheap) or *source* onsets.
  Deriving the onset from a mid-chain *transformed* `NoteStream` has the same
  earlier-tick upstream-lookback problem as Delay, so that variant is gated on
  `eval_prefix_at_tick` and deferred with the timing nodes (§9).
* **`StepLfo`**: step-sequenced `Value`; pointer = tick arithmetic.

### D. Utility & Routing — **earned escalation (DAG)**
These need a branched/merged `NoteStream` and are therefore built with the DAG,
not in v1 (§0.1). Listed here so the escalation has a spec.
* **`KeyZoneSplitter`** / **`VelocitySplitter`**: route notes to output A/B by
  pitch/velocity range. Pure filters.
* **`Alternate / RoundRobin`**: routes the *k*-th onset to output `k % n`;
  ordinal derived by a bounded scan of upstream onsets (§1.1).
* **`ProbabilityGate`**: seeded-hash pass/block per note occurrence
  (`splitmix64(seed ^ host_key ^ note_event_key ^ tick)`, where `note_event_key`
  is the §0.8.1 runtime key), reproducible like Humanize. **It must roll and drop
  itself** — writing `NoteExpression::probability` is useless because the engine
  resolves probability on *source* notes only, before the graph runs (§4,
  §0.8.1), so a generated note's field is never re-gated. Optional `Value` input
  modulating the threshold. *(A pure pass/block filter with a single in/out is
  linear-safe and could ship earlier; its splitter cousin is the DAG part.)*

**Routing scope**: even with the DAG, the graph has exactly one terminal
`NoteStream` output, feeding the host's instrument — same destination contract
as the rack. Routing different splitter branches to *different
instruments/tracks* crosses the pattern→track binding and is explicitly **out
of scope** (splitters are for building different processing per branch, then
merging).

**Merging** *(DAG)*: a `NoteStream` input port accepts multiple connections;
streams concatenate in deterministic (connection-sorted) order, subject to the
buffer cap. This is the merge semantics deferred with the DAG.

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

### 6.1. `NoteScriptTransform` (dialect: `note_event`) — a v1-era module

Runs once per note event, 1-to-1, stateless. New grammar unlocked by the
dialect (everything stays `f32`, NaN-free, same VM):

- **Reads**: `note_pitch` (raw MIDI 0–127, matching `mtof`'s convention),
  `note_vel` (0..1), `note_dur` (ticks; `-1` sentinel = `None`, "plays until
  cut"), `tick`, plus `in1..in4` (`Value` ports).
- **Writes** (statements, like the audio dialect's `out.left`): `out_pitch`,
  `out_vel`, `out_dur`, `out_gate`. Unwritten fields pass through. **Sentinel checks run first, before any
  clamping** (a clamp would otherwise erase the sentinel): a negative `out_vel`
  drops the note, and a negative `out_dur` restores `None` ("plays until cut").
  Surviving positive values are then clamped (so scripts can't produce illegal
  notes): `out_pitch` rounds to nearest and clamps to `0..=127`; `out_vel` to
  `0.0..=1.0`; `out_dur` to `≥ 0`; `out_gate` to the gate-fraction range. **There is no `out_prob`** — probability is not a
  writable field (§0.8.1, §5.D `ProbabilityGate`): the engine only gates source
  notes, so a script that wants probability must roll its own seeded drop (i.e.
  use `out_vel = -1` behind a seeded hash) or a downstream `ProbabilityGate`.

```yams
# Transpose an octave; LFO from in1 shapes velocity.
out_pitch = note_pitch + 12
out_vel = note_vel * (0.5 + 0.5 * in1)
```

Because the VM is deterministic and per-event state resets, this module is
pure by construction. Compile on the UI/MCP thread, run bytecode on the audio
thread — the existing YAMS execution model. This is a 1-in/1-out stream
transform, so it is linear-safe (fits v1's constraint).

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
  graph itself survives; other hosts may use it). **This method lives on `Song`,
  not `Pattern`** (unlike today's `Pattern::freeze_processors`): the graph is in
  the `Song` pool, so a `Pattern` method cannot reach it — freeze needs
  `&Song` to resolve the id and the host key. Purity makes the bake identical to
  playback by construction. The look-back window extends the walk range the same
  way `strum_tail` does today. With one graph per scope (§0.3) the bake target is
  unambiguous: bake the note-scope then pattern-scope result for the host being
  frozen.
- **Tracker**: the rack's per-processor columns (`expand_at_tick_through`)
  don't generalize to a graph the same way. v1: tracker shows a single "graph
  output" column (like the existing read-only NP output column); per-node taps
  (pick a node, render its output buffer) are a later GUI feature and are part
  of what gates rack retirement (§1.3).
- **MCP**: pool CRUD + reference setters —
  `list_note_graphs` / `create_note_graph` / `delete_note_graph` (returns the
  usage list), `add_note_graph_module`, `connect_note_graph` (validates the
  linear-stream rule and lists valid ports on error),
  `set_note_graph_module_param`, `get_note_graph`, and
  `set_pattern_note_graph` / `set_note_note_graph` (array-taking variants per
  the MCP array-tools convention; descriptor-driven validation).
- **Offline analyzers** — *SHIPPED for the pitch-domain analyzers*. The "they
  share `expand_at_tick`, so this falls out" assumption was **false** for the
  note-domain analyzers: they read `pattern.notes()` (the raw source), ignoring a
  bound graph. `Song::expanded_pattern_notes(pid, bpm)` (a non-mutating expansion
  via `bake`) now exists, and **`analyze_pattern` / `analyze_harmony` /
  `analyze_harmonic_function`** route through it — both their pattern-scope and
  arrangement-scope note collection (expanded notes stay pattern-local, so the
  placement offset + transpose apply unchanged; expand-then-transpose is an
  accepted approximation). A `[0,4,7]` Chord node reads as the triad end-to-end
  through MCP (regression-tested). The **drum/timing analyzers**
  (`analyze_drum_groove`, `analyze_bass_drum_lock`) still read source — low value
  (drums are rarely note-graphed), left as-is. Audio analyzers (`analyze_section`)
  already reflect the graph — they render through the engine.
- **Undo**: graph edits go through the same undo path as pattern edits; note
  that undoing an edit to a shared graph affects all hosts, same as
  instrument-patch undo.
- **Dropped-event reporting** — *SHIPPED at the freeze boundary*. `bake_counted`
  returns `(notes, dropped)`; `Song::freeze_pattern` returns a `FreezeStats`; the
  count surfaces via MCP `freeze_pattern` (`dropped_events` + `warning`) and the
  GUI activity log. A *live per-node canvas* variant (the original wording) needs
  RT-evaluator per-node attribution plus a new engine→GUI sync channel for a
  near-never-hit 128-event cap — deliberately not built; the freeze boundary
  (non-RT, real notes, at commit time) is the right-sized surface. *Follow-up if
  ever wanted*: live per-node badges.

---

## 8. UI/UX Integration (egui) — mirror the instrument view

The guiding decision (§0.6): **the Note Grid view mirrors the instrument (Rack)
view** so users recognize the layout across views. The Rack view is
`AppView::Rack` → left `render_instruments_panel` (instrument list) + central
`render_rack_central` (the active **egui::Scene** patch editor).

### 8.1. Note Grid view — new `AppView::NoteGraph` tab

```text
 topbar:  Home │ ⊞ Rack │ ⋔ Note Grid │ Pattern │ Sequencer │ Mixer │ Sample
         ────────────────────────────────────────────────────────────────────
┌─ left SidePanel ────────┐ ┌─ CentralPanel: Scene node editor ────────────────┐
│  Note Graphs      [ + ] │ │  ⋔ "Trance Arp"    used by 3 · [make unique]     │
│ ┌─────────────────────┐ │ │  ┌────────┐  ┌───────┐  ┌────────────┐           │
│ │▉ Trance Arp       ◀─┼─┼─┼─│ Scale  │─▶│ Chord │─▶│ Arpeggiator│─▶ out      │
│ │▉ Humanize Soft      │ │ │  └────────┘  └───────┘  └─────┬──────┘           │
│ │▉ Echo 1/8 dotted    │ │ │                       ┌──────┴─────┐             │
│ │▉ Euclid 5/8         │ │ │                       │  Note LFO  │ Value→rate  │
│ └─────────────────────┘ │ │                       └────────────┘             │
│  create/dup/delete,     │ │                                                   │
│  rename, color          │ │                                                   │
└─────────────────────────┘ └───────────────────────────────────────────────────┘
   = render_instruments_panel-analog      = reuses the shipped egui::Scene patch editor
```

- **Left panel = the pool**, built as the `render_instruments_panel`-analog:
  name, color swatch, description, usage count ("3 patterns, 12 notes"), create
  / duplicate / delete / rename / recolor, click-through to load the graph in
  the center. This is the overview the pool model buys. Metadata + lifecycle
  only — **not** a second content editor.
- **Center = the egui::Scene node canvas**, reusing the patch editor's
  `ModuleNode`, port and cable rendering
  ([`patch_editor/`](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/patch_editor)),
  with the Note Grid's own port-type palette (`NoteStream` / `Value` / `Gate`).
  **This is the single content editor.** In v1 the stream axis is validated
  linear, so the canvas shows one left→right chain plus `Value`/`Gate`
  modulation cables into params; the Scene machinery is DAG-ready for free (§0.6
  inverts the old "chain-list first" ordering — the compact rack-list is an
  optional later alternate).
  - **Reuse is an adapter/refactor, not free.** Today's `PatchEditor`
    ([`patch_editor.rs`](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/patch_editor.rs))
    is welded to the audio graph: it keys nodes by `synth_core::ModuleType`, is
    namespaced by `instrument_id`, and special-cases `ModuleType::ModMatrix /
    Script / AudioScript`. **Phase 5 (§9.5, shipped) extracted only the leaf
    layer** — `scene_canvas` (pan/zoom canvas, camera, view controls) plus the
    shared `ModuleFrame` / `PortWidget` / cable painters / `ModuleWidth` — and
    gave the Note Grid its *own* node loop, port column, cable drawing, and wire
    FSM. **Fully converging those two parallel implementations into one generic
    node-canvas is its own phase (§9.6, next)** — lifting the node-render+drag
    loop, port geometry, cable render/hit-test, and the connect FSM behind a
    node-type trait so both editors share them.
- The four algorithm nodes (`ScaleQuantizer`/`ChordGenerator`/`Arpeggiator`/
  `Humanize`) reuse the existing processor configs and the note-FX card bodies
  from
  [`sequencer/note_fx.rs`](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/sequencer/note_fx.rs)
  as their node-body content.

### 8.2. Host binding UI

- **Pattern editor**: a plain **dropdown** (`Choice`) to bind one graph to the
  selected pattern — the track-instrument-selector analog — plus an "edit"
  affordance that jumps to the Note Grid view with that graph loaded (the second
  entry point into the one content editor). No add/remove/reorder here; node
  ordering lives in the Note Grid view.
- **Note inspector** (later phase, §2.1): the same single-graph selector per
  note in the piano roll / tracker note inspector.

### 8.3. Visual feedback (optional polish, not v1)

Cables pulse when events pass. Requires audio→UI telemetry (per-edge event
counters over the existing `EngineEvent` ring, throttled).

---

## 9. Phasing

The order below reflects the linear-first, Scene-node-view decisions (§0). v1 is
phases 1–5 (**shipped**); everything after is progressive extension, and each
phase leaves the rack working so it can be abandoned at any boundary. Phase 6
(**next**) converges the two node-canvas implementations *before* any further
note features, so the graphics are one hardened, shared surface first.

1. **Evaluation core — standalone spike** (`synth_sequencer`): `NoteGraph`,
   `NoteModuleId`, `NoteEventKey` (§0.8.1), topo order (linear case),
   `NoteGridScratch`, pure per-tick evaluation with host-key seed mixing; nodes
   wrapping the four existing processors; the linear-stream connect-time
   validation. **Kept as standalone types — NOT yet added to `Song`** so this
   phase truly has no serde/schema surface (§0.8.3). **Unit tests are the gate**:
   seek-order-independence, bake-equals-playback, and shared-graph decorrelation.
   Built and green before anything else. (The throwaway-able spike that proves
   the fundament; §0.7.)
2. **Persistence + engine** (pattern scope): wire the pool into `Song`
   (`note_graphs` + `next_note_graph_id`) and add `Pattern::note_graph`, both
   with `#[serde(default, skip_serializing_if = …)]`; re-run `gen_schemas` and
   commit `project.schema.json` (§0.8.3); `NoteGraph::rebuild_derived()` on every
   load/mutation path (§1.2). Then the expansion path, dangling-reference
   pass-through, freeze on `Song` (§7), drop-counter plumbing.
3. **Module catalog + modulation (linear-safe only — v1)**: the
   source-independent `EuclideanGenerator`, the `Value`/`Gate` sources (`NoteLfo`,
   `StepLfo`, and `NoteEnvelope` on a `Gate`/source onset only, §5.C) with param
   modulation wiring + the cycle-inclusive, latency-free validation (§0.8.4), a
   pass/block `ProbabilityGate` that rolls itself (§5.D), and `NoteScriptTransform`
   (§6.1). **Timing nodes (Delay/Echo/Ratchet) are NOT in v1** — they are blocked
   on the `eval_prefix_at_tick` look-back spec; see the deferred phase below.
4. **MCP surface** + analyzer regression tests.
5. **GUI**: extract the generic Scene layer out of `PatchEditor` (§8, the
   `ModuleType`/instrument coupling is real work, not free reuse), then the
   `AppView::NoteGraph` view (pool list + Scene node canvas) + the pattern-editor
   dropdown; tracker single graph-output column.
6. **Shared node-canvas extraction — graphics convergence** (do this *next*,
   before the rest, so both editors share one hardened canvas). Phase 5 shipped
   only the **leaf** layer that is shared today —
   [`scene_canvas`](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/scene_canvas.rs)
   (grid / snap / screen↔world / the `egui::Scene` builder / camera framing /
   view controls), the `ModuleFrame` + `draw_module_header`/`footer` chrome,
   `PortWidget`, the `widgets/cable.rs` painters, and `synth_core::ModuleWidth`.
   But the Note Grid got its **own** node-render+drag loop, port-column, cable
   drawing, and wire drag-to-connect FSM, running *parallel* to `PatchEditor`'s
   (`patch_editor/{node,ports,wiring}.rs`) — so a bug or feature in porthit-test
   or wire-dragging must be fixed in both. This phase lifts those into a
   **generic node-canvas** both editors instantiate, parameterized over a
   node-type trait: a `NodeId: Copy + Eq + Hash`, a node descriptor (ports =
   name / kind / direction, accent, `ModuleWidth`), a port-kind
   (`color()`/`shape()`/`can_drive()`), a body callback
   (`FnMut(&mut Ui, NodeId)`), and a deferred **result** type (the existing
   `PatchEditorResult` pattern). Shared then: the node loop
   (position / drag / measure), the IN | content | OUT port frame + anchor
   recording, cable render + hover hit-test, and the pending-wire → drop FSM —
   so the Note Grid's hardening (primary-button-only drags, canvas-confined
   drops, occupied-port-aware highlight, `propose_edit` priority) and the patch
   editor's own fixes land **once**, not twice. **Stays view-specific** (not
   forced into the generic layer): PatchEditor's groups / collapsed boxes,
   effect-chain cables + category zones, mod-matrix / script markers,
   connectivity / bypass status, and the `ModuleType` body dispatch; the Note
   Grid's active-spine / off-spine-inert semantics and its note-FX node bodies.
   `PatchEditor` is the reference (behaviour preserved, screenshot-verified);
   the Note Grid is the second consumer that proves the generic seam. Real,
   careful work — the patch editor's version is entangled with all of the above,
   so extract **incrementally**, one shared piece at a time, with the full gate
   green and both editors eyeballed at each step.
   **Status — the high-value core is converged (branch `feat/note-canvas-convergence`).**
   The genuinely-duplicated, high-risk-if-divergent piece — the **drag-to-connect
   wire FSM** — now lives once in `gui/node_canvas/wiring.rs` (`WireDrag` /
   `WireEvent` / `DropTargets` / `resolve_wire_events` / `push_port_event`),
   generic over the caller's `Port: Copy + Eq + Hash` and connection type via
   closures. **Both editors drive it**: the Note Grid (P6.1) and the patch editor
   (P6.2, which *gained* drag-release-to-connect on top of its click-click, wire
   state moved out of `CanvasInteraction` into a standalone `pending_wire`,
   `open_patch_connection` supplying the audio-graph validation), and the
   port-response→event idiom is shared too (P6.3). The Note Grid's hardening
   (primary-button drags, canvas-confined drops, occupied-port-aware highlight)
   now applies to both. **Deliberately left view-specific** (converging them would
   be over-abstraction — the plan's own "stays view-specific" warning): the
   *node-drag loop* (patch multi-select `DraggingNodes` vs Note Grid single-node —
   the shared atom is a one-line `ui.interact`), the *cable orchestration* (patch
   fg-glow sublayer + per-destination fan-out + groups + effect-chain vs the Note
   Grid's simple loop — the cable *painters* and hit-test are already shared leaves
   in `widgets/cable.rs`), and the *port-column layout* (patch vertical IN|OUT
   rails vs the Note Grid's compact rows — the `PortWidget` + anchor + emit atoms
   are shared). No full generic node-canvas *widget* / node-type trait was built;
   the shared surface is the wire-FSM free functions, which is where the real
   duplication was. **Patch-editor in-app eyeball done (2026-07-10, via egui MCP
   on the Vintage Electric Piano patch).** Existing cables render clean;
   click-click wiring still arms the output port and lights compatible input
   ports as valid targets before the second click completes it; and the newly
   *gained* drag-release-to-connect draws a fresh cable on drop. No regression —
   the only oddity, a faint cross-canvas out→FX cable that flickers at some zoom
   levels, is a pre-existing long-cable visibility artifact in the already-shared
   `widgets/cable.rs` painters, untouched by P6.
7. **NoteScriptTransform** (YAMS `note_event` dialect), if not folded into 3.
8. **Note scope**: `Note::note_graph`, collection-time evaluation (ornament
   seam), inspector selector UI.
   **Status — the engine half shipped (note-scope "i", branch `feat/note-scope`).**
   `Note::note_graph` now evaluates during source seeding: `seed_source_at_tick`
   resolves each note's bound graph and expands that one note through it (host =
   NoteId, note scope disabled inside so no recursion; dangling/`None` = plain
   pass-through), splicing the result into the seeded stream so the pattern-scope
   chain sees the articulated notes. Threaded via `NoteScopeCtx { pool, scratch }`
   (its gate is `&dyn Fn` to close the re-entrant monomorphization cycle). Wired
   into both engine collect branches (arrangement + preview) with a dedicated
   `scratch_note_scope`; into freeze/bake (rack path + `freeze_pattern_note_graph`,
   walk extended by the note-scope articulation tail, baked notes retire the
   binding); and MCP `set_note_note_graph` (+ `NoteInfo.note_graph` read-back).
   Tests: single-note articulation, note→pattern chaining, NoteId decorrelation,
   dangling/None pass-through, seek-order independence, freeze==playback (both
   paths), engine playback + preview, MCP bind/clear/validate round-trip.
   **Note-scope "ii" SHIPPED (branch `feat/note-grid-p8-p9`).** The piano-roll
   selection inspector gained a per-note graph selector (combobox + jump,
   multi-select with a `SetNoteGraphBindingBatch` batch undo,
   `Pattern::set_note_note_graph`). Both GUI previews now pass a real
   `NoteScopeCtx`: `compute_ghosts` (+ the ghost-cache key now folds in the bound
   note-scope graphs' content, compared by reference and cloned only on a miss)
   and the tracker `compute_np_stages`. Phase 8 is complete.
9. **Rack retirement — SHIPPED (branch `feat/note-grid-p8-p9`).** Full retirement,
   per the user's decision. The per-node tracker taps and rack→graph migration
   were already done (tracker uses `expand_at_tick_tapped`;
   `migrate_processor_racks_to_graphs` runs at load), so retirement removed the
   *authoring surface*: the Note FX panel's rack list + Add/Remove/Set (it now
   only binds a graph + freezes), the tracker rack per-processor columns, the
   rack undo actions (Add/Remove/SetNoteProcessorConfig), and the four rack-CRUD
   MCP tools (`freeze_note_processors` was renamed `freeze_pattern` and kept —
   it bakes the graph). `Pattern.processors` is now a **deserialize-only legacy
   field** (kept `#[serde(default)]` for pre-migration projects; `add_processor`
   / `clear_processors` / `processors()` survive for the migration + freeze +
   fixtures; `set_processor` / `remove_processor` deleted). `NoteProcessor` +
   `process_at_tick` stay — shared with the graph's `Processor` node. The `edit_*`
   note-FX card bodies stay — reused by the Note Grid nodes.
10. **Earned escalation — DAG**: relax the linear-stream validation; splitters
    (`KeyZoneSplitter`/`VelocitySplitter`), `RoundRobin`, merge semantics
    (connection-sorted concat), held-pitch resolution through branched upstream.
    Only if real usage shows branching is needed.
10. **Earned escalation — DAG**: relax the linear-stream validation; splitters
    (`KeyZoneSplitter`/`VelocitySplitter`), `RoundRobin`, merge semantics
    (connection-sorted concat), held-pitch resolution through branched upstream.
    Only if real usage shows branching is needed.

    **Cost / benefit (why this is deferred, not planned).** The intuition that
    the DAG "needs a lot of engine work" is only half right — most of the
    infrastructure already exists; the genuinely-new work is narrow but thorny,
    and the payoff is speculative.

    - *Already free (built/pre-provisioned in v1):* the DAG is **relaxing the
      linear validation, not a rewrite** (§0.1, §1.3). The data model is already
      graph-shaped (`nodes` + `connections` + `processing_order` via Kahn sort),
      so **no serialization change and no re-plumbing**. `NoteGridScratch` already
      owns *one buffer per node* (`MAX_NOTE_GRID_NODES = 32`, §1.4), so a branch
      that needs its own downstream buffer is already accommodated. The pure
      per-tick evaluation core is **identical** for linear and DAG (§0.7, §1.1) —
      purity is not the obstacle.
    - *Genuinely new internal work:* (a) **merge semantics** — a `NoteStream`
      input accepting multiple connections, concatenated in connection-sorted
      order under the buffer cap (§5); moderate. (b) **branch evaluation + the
      relaxed connect-time rule** — small. (c) **held-pitch resolution through a
      *branched* upstream (§5.B) — the hard part.** The held-pitch seam
      ([`expand_pitch`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_processor.rs#L1072))
      resolves a downstream arpeggiator's held tones through a **single linear
      upstream chain** (the rack's `upstream: &[NoteProcessor]` slice). With a
      branch there is no single "the upstream" — resolving held pitch through a
      branched/merged upstream needs **new semantics**, and that is the piece that
      makes this real engine work rather than a validation flag.
    - *Why the value is low (so it stays gated):* the graph always has **exactly
      one terminal `NoteStream` output** and **cross-instrument routing is
      explicitly out of scope** (§5) — a splitter therefore *cannot* fan branches
      to different instruments/tracks; its only use is "build different processing
      per branch, then merge back to the one output". That is a narrow need, and
      paired with the held-pitch complexity the cost/benefit is poor **until a
      concrete use case appears**. Hence "only if real usage shows branching is
      needed", not a scheduled phase.
11. **Deferred timing modules — SHIPPED, squashed into `feat/note-grid`, eyeballed**:
    the §0.8.2 `eval_prefix_at_tick` look-back model (bounded prefix re-eval,
    recursion budget as a consumable scratch slice, hard repeat/depth caps, gate
    threading — the pre-implementation `note-grid-timing-lookback.md` spec is now
    fully in code and was removed), then `NoteDelay`/`Echo`, `Ratchet`, and the
    opt-in transformed-`NoteStream` (`StreamOnset`) `NoteEnvelope` variant on top of
    it. Full adversarial review (RT-safety / termination / purity / freeze==playback)
    passed; caps cut to `repeats/subdiv 16`, `depth 2` to bound `R^depth` cost. In-app
    eyeball found + fixed a `StreamOnset` RT stall (dry-only probes) and a
    `duplicate_pattern` binding drop.
12. **Later**: track scope (merged-stream evaluation, cross-placement look-back,
    freeze story); NoteScriptGenerator (`emit`); MicrotonalTuner (per-note
    detune field + event plumbing); per-reference overrides; per-scope `Vec` of
    graphs; cross-track routing; cable telemetry; per-node tracker taps.

---

## 10. Improvements (Note Grid GUI polish — parity with the patch editor) — SHIPPED

**All four items below shipped, squashed into `feat/note-grid` (@`ed034cb6`),
in-app eyeballed.** A follow-up round then took the node view to full rack
parity: the node now uses the rack's real `IN | body | OUT` port-column layout
(`draw_note_port_column`), node accents are keyed to the rack module-category
palette (with a colour-coded add menu), the script node uses the rack's ƒx
button + source preview, and the three script popups share one
`gui/script_editor` surface (code field + compile status + wrapping header +
`note_event` Help window). The original four items:

The Note Grid node canvas shipped with its own compact node/port rendering
(`gui/note_grid_view.rs`), so several affordances the audio patch editor has
were never carried across. These are polish items, not new features — the goal
is that a node in the Note Grid reads and behaves like a module in the Rack.

1. **Missing `IN` / `OUT` port-column headers.** The patch editor prints a small
   `IN` / `OUT` label plus a vertical rail above each port column
   ([`draw_port_column_with`](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/patch_editor/ports.rs#L250)).
   The Note Grid draws ports as inline horizontal rows (`draw_node_ports` in
   `note_grid_view.rs`) with **no such header**, so the node's in/out sides are
   unlabeled. Add the equivalent `IN` / `OUT` cue to the note-module ports.
   (Note: the Note Grid layout is horizontal rows, not the patch editor's
   vertical `IN | body | OUT` rails, so this is an *equivalent affordance*, not a
   verbatim copy — e.g. a dim `IN`/`OUT` caption on the port row, or adopt the
   rail layout.)

2. **Weak / missing widget tooltips.** Patch-editor ports get a rich lazy
   `on_hover_ui` tooltip — port name + signal type + the longer description
   ([`ports.rs`](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/patch_editor/ports.rs#L327)).
   Note Grid ports only get a bare `on_hover_text(label)` ("notes in", "mod in",
   …) in `port_widget`, and the **node-body widgets** (knobs / drag values in the
   note-FX card bodies) have essentially no tooltips. Give every note-module port
   and body widget a good hover tooltip matching the patch modules (name + type +
   what it does / units / range).

3. **Script mod inputs are unnumbered.** `NoteScriptTransform` exposes four
   `Value` inputs mapping to `in1..in4`
   ([`value_input_count`](file:///home/per/github/pertylizer/crates/synth_sequencer/src/note_graph.rs#L728),
   `MAX_VALUE_INPUTS = 4`), but `draw_node_ports` labels **every** value input
   generically ("mod in" hover + a dim `mod` caption), so on a script node the
   four ports are indistinguishable. Number them to match the register names the
   script reads — `in1` / `in2` / `in3` / `in4` (or `mod 1`…`mod 4`) — on both the
   caption and the hover text, so a cable's target register is unambiguous.

4. **Script editing should use the shared ƒx popup, not an inline text box.**
   The two Rack script modules (YAMS `Script` + `AudioScript`) edit their source
   in a shared **ƒx expression-editor popup** — `egui::Window` with live compile
   status + Apply/Clear, driven by `ScriptEditorState`
   ([`module_panel.rs`](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/module_panel.rs#L63))
   and
   [`patch_editor/popups.rs`](file:///home/per/github/pertylizer/crates/pertylizer/src/gui/patch_editor/popups.rs#L152).
   The Note Grid's `NoteScriptTransform` node instead edits inline via a
   multiline `TextEdit` (`edit_note_script` in `note_grid_view.rs`). Give the note
   script node the **same ƒx button → same popup**. The popup already
   distinguishes dialects (audio vs mod); **generalize it to also serve the
   `note_event` dialect** (the note script compiles via
   `session::compile_note_event_script`) so all three script variants share one
   editor. This kills the divergent inline editor and gives the note script the
   Rack's compile/Apply/Clear UX for free.
