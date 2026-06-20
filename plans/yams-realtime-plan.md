# Plan: YAMS as a general real-time language

Today **YAMS** (`docs/yams.md`) is a control-rate expression language bound to
**Mod Matrix slots**: per voice, once per control block (`sr/block_size`,
~187–750 Hz), each slot computes a single `out` that becomes a normalized
additive parameter offset. This plan generalizes that one engine into a
real-time computation language usable in many places.

Direction chosen: **A + E first** (cheap, broad generalization of the engine
that already exists), then **B** (audio-rate DSP) as a self-contained later
stage. **C is dropped** — see the Phase 3 note below for why; in short, Phase 1
(E) already delivers the bulk of "global" modulation for free, so C is the
heaviest lift in the plan for the narrowest unique payoff.

- **A** — generic control-rate modulator as a first-class graph node.
- **E** — new sources (transport / tempo / more params).
- **B** — audio-rate, per-sample scriptable DSP module.
- ~~**C** — global / instrument-scope scripts + a shared modulation bus.~~
  **Dropped** (do reactively + minimally if a real patch ever demands it).

---

## 0. Design foundation (governs the whole plan)

Three facts from the code drive every decision below:

1. **The VM is already rate-agnostic.** `eval()` in
   `crates/synth_core/src/script/eval.rs` drives `lag`/`slew`/`phasor` through
   `dt = 1/control_rate`. Feeding `sample_rate` instead of `sr/block` makes the
   time semantics work unchanged at audio rate. Nothing in the bytecode is tied
   to control rate.

2. **The only coupling to the Mod Matrix is *where* eval runs and *what* `out`
   is written to.** Today: `voice.rs::resolve_routings_into_cache` resolves
   sources → `eval()` → `apply_mod_offset_addr`. Everything else (bytecode,
   `RegisterFile`, source resolution) is already generic.

3. **The borrow conflict is the crux.** `eval()` wants `&graph` (to read sources)
   and `&mut regs` at the same time, and the module *lives in* the graph. That is
   why the Mod Matrix pulled `script_regs` out to `Voice`. We dissolve it
   generally with **two-step resolve→eval** (resolve sources into a scratch
   buffer *first*, drop the graph borrow, then eval), so state can live in the
   module and any module can host scripts.

### Cost model (the one knob that matters)

```
core-fraction ≈ (evals/sec) × (ops/eval × ns/op) × (concurrent instances) / 1e9
```

- `evals/sec` = `control_rate` (~187 Hz) **or** `sample_rate` (48 000 Hz) — a
  ~256× factor. This is the only order-of-magnitude lever.
- `ns/op` ≈ 1–3 ns (jump-table dispatch + a few L1 reads). Typical 25-op script
  ≈ ~50 ns; heavy 200-op script ≈ ~400 ns.
- `instances` = voices × script-slots.

Consequence: **A/E (control-rate) are effectively free**; **B (audio-rate)
scales with voices and must be budgeted** (heavy stereo script × full polyphony
≈ half a core per instance). Numbers are a hypothesis until Phase B benchmarks.

---

## Phase 0 — Foundation: make script execution module-agnostic

**Goal:** zero behavior change; the Mod Matrix works exactly as today, but the
script machinery is no longer Mod-Matrix-specific.

- Introduce a small `ScriptHost` abstraction (trait or helper) owning:
  `slots: [Option<Arc<BoundScript>>; K]` + `regs: [RegisterFile; K]` + a
  `[f32; MAX_SOURCES]` scratch per slot.
- **Two-step in `voice.rs`:** a `resolve_script_sources` pass that, for *each*
  script-bearing module, resolves `BoundScript.inputs` (macros, context,
  address sources) into the module's scratch while holding only `&graph`. Then
  the graph runs normally and each module `eval`s from its scratch + its own
  `regs`.
- **Move `RegisterFile` into the modules** (they are already cloned per voice →
  each voice clone gets its own state, reset on note-on). Removes the Voice-side
  `script_regs` special case.
- **PRNG decorrelation — `PolyModule::set_voice_index(&mut self, u32)`** (default
  no-op). Moving `RegisterFile` into the module loses the voice ID that
  `voice.rs::script_seed_index` currently supplies, so simultaneous voices would
  run phase-locked PRNG streams (broken stereo spread). Propagate it in
  `Voice::from_graph` (`voice.rs:514`) — iterate `graph.module_ids()`,
  `graph.get_module_mut(id).set_voice_index(id.as_u32())` — store it in the
  module, and reseed from the *stored* index on note-on (not from a stale value
  after voice stealing). (All three APIs already exist.)
- **Pre-existing `Arc` deallocation hazard — ALREADY FIXED on `main`** (ahead of
  this plan). `SetModScript` is drained in `process_commands()` **on the audio
  thread** (`synth_engine.rs::handle_set_mod_script`), where the replaced
  `Arc<BoundScript>` was dropped inline → a possible `free()` in the audio
  callback. Fix: `PolyModule::set_mod_script` now **returns** the replaced script
  (`#[must_use]`), and the engine routes it through a new `script_trash`
  ring (mirroring `automation_trash`) so the final drop runs on the main thread
  via `EngineHandle::cleanup_dropped_modules()`. No new dependency. The `Arc`
  rides a **dedicated typed channel**, *not* the sequencer's `AutomationTarget`
  enum — keeping the compile/script layer out of the track/song layer (the unused
  `DroppedItem` enum is dead code; the live convention is one typed ring per
  dropped type). When this phase generalizes `set_mod_script` → `set_script`,
  keep the return-the-replaced-`Arc` contract.
- The Mod Matrix becomes the *first* consumer of `ScriptHost` instead of being
  hardcoded.

**Files:** `voice.rs`, `mod_matrix.rs`, new `synth_core/src/script/host.rs`,
`module_traits.rs` (generalize `mod_scripts`/`set_mod_script` → `scripts`/`set_script`;
add `set_voice_index`), `synth_engine.rs` (script-Arc trash channel).
**Test:** existing mod-matrix tests green; new unit test that resolve→eval yields
identical results to today's path; a two-voice PRNG-decorrelation test.
**Pure refactor + one RT-safety fix; no functional change.**

---

## Phase 1 (E) — New sources: transport, tempo, more params

Cheapest, purely additive, unlocks tempo sync everywhere.

- New `Context` variants in `crates/synth_script/src/symbols.rs`: `beat`,
  `bar_phase` (0..1 within the bar), `tempo` (Bpm normalized), `playing`
  (transport running). **The data is already in `ProcessContext`** —
  `position_beats: BeatPosition` (`module_traits.rs:329`, with `.beat_in_bar()`/
  `.fraction()`) plus `tempo: Bpm` — so this is pure read-out, no new plumbing.
- **Newtypes at the resolve boundary, not in the VM.** Inside the bytecode
  everything is `f32` by design (`docs/yams.md`). The newtypes (`BeatPosition`,
  `Bpm`, `NormalizedValue`) apply where the engine reads transport and writes the
  source register; normalize to `f32` there.
- **Parameter sources resolved at bind time, not per block.** Reading
  `flt-1.cutoff` (not just output ports) is descriptor-driven, but do the
  descriptor lookup **once in `into_bound`** and store a fixed accessor — the
  per-block path must stay a direct read with no string matching. Concrete shape:
  a new `ScriptInput::ModuleParam { module_id, param }` variant where `param: Param`
  is the concrete descriptor placeholder resolved at bind time; the voice loop
  then reads it via `graph.get_param(module_id, &param)` (`graph.rs:392`, already
  exists) — O(1), no string scan. General rule for the whole plan: every named
  source (macro, context, module port/param, later the global bus) is pre-bound to
  a fixed accessor/index; **no string compares in the voice loop.**
- `eval_ctx` carries the transport fields.

**Files:** `symbols.rs`, `compile.rs` (source allocation), `bound.rs`
(`ScriptContext`), `voice.rs` (resolve), `docs/yams.md`.
**Test:** `out = sin(beat * tau)` tracks tempo; `playing ? x : 0` mutes on stop.

---

## Phase 2 (A) — The Script module

The headline of A+E: scripted control signals as first-class graph nodes —
reusable, shared, chainable.

- **`ModuleType::Script`:** a rack of **K=8** slots, each slot =
  `Option<Arc<BoundScript>>` + `RegisterFile` + **one output port** `out1..outK`.
  (Multi-out *without* a grammar change: K output ports = K single-`out` scripts.
  This defers the big grammar work to Phase B, where stereo genuinely needs it.
  K=8 costs ~640 B state/voice — trivial — and control-rate eval makes the CPU
  cost of 8 slots negligible.)
- `process()`: eval each slot from resolved scratch + its own `regs`, then
  **`buf.fill(out)` over the *entire* output buffer**, not just `buffer[0]`. A
  port consumer (FM input, filter input) reads the whole slice, so filling only
  `buffer[0]` would give zipper noise / silence; a 64–256-float fill is free.
- **Source model: address-based** (recommended) — keeps YAMS's core value
  "reference anything by name" (`src lfo = lfo-1.out`), reuses `resolve_source`,
  inherits the 1-block latency (harmless for modulation). The output is a *real*
  port → downstream via normal graph routing / topological order. Consumed
  either as a Mod-Matrix source (`scr-1.out`) or via a port connection.
- **Why not just a Mod-Matrix slot:** the module computes the value *once* and
  can be referenced by many slots/modules (shared computation), chained (a script
  reads another script's output port), and saved as a reusable building block (a
  custom LFO wired in several places).

**Files:** new `synth_modules/src/script_module.rs`, registration in
`synth_modules` + `synth_core` descriptor, `synth_mcp` (generalize
`set_mod_matrix_script` → `set_script` with `module_id`), GUI (reuse the ƒx
editor from the Mod Matrix for K slots).
**Test:** script module drives filter cutoff via a Mod-Matrix source; chain
script→script; bundled example patch.

---

## Phase 3 (C) — Global / instrument scripts + global mod bus — DROPPED

**Decision (2026-06-20): not building this.** Kept here as a record of why, plus
the design sketch for a minimal future slice should a real patch ever demand it.

**Why dropped — Phase 1 already delivers most of the payoff for free.** A
*stateless* expression over shared transport context (`beat`, `tempo`,
`bar_phase`, `playing`) yields a **bit-identical** result in every voice on its
own — same inputs, same pure function. So the canonical "global" cases already
work today, per-voice, with no new machinery:

- tempo-synced LFO `sin(beat * tau)` — identical across voices ✓ (Phase 1)
- rhythmic pump from `bar_phase`, tempo→rate `tempo / 60` — stateless ✓
- script→script chaining — already works per voice ✓ (Phase 2)

What C would *uniquely* add is narrow: (1) shared **stateful** evolution across
voices — one `rand()`/accumulator all voices read the *same* value of (per-voice
`rand` decorrelates by design and can't be faked); (2) state that **survives
note boundaries** (a global LFO that doesn't reset on each note-on); (3) CPU:
eval once vs N. Point (3) is negated by this plan's own cost model (control-rate
eval is "effectively free", so N× redundant identical math at ~187 Hz is
near-zero). That leaves a musically-real but **niche** payoff (shared dice-roll,
note-independent free-run) against the **heaviest lift in the whole plan**: it is
the only phase that breaks the per-voice-clone invariant, touches the hot
`ProcessContext`, adds an RT write-before-read ordering rule, a persistent
name→index registry on `Instrument`, and save/load + MCP + GUI surface for an
instrument-scope rack. Worst cost/payoff ratio in the plan → defer.

Honest counter-point (recorded, not decisive): today "global" modulation works
only *by coincidence* (stateless + shared input), and that illusion **breaks
silently** the moment a user adds any stateful op or per-voice macro. C would
turn that accident into a guarantee. Real, but a robustness argument — not enough
to justify the largest build speculatively.

**If revisited, build the minimal slice, not the full rack:** just the
`glob`-bus + a *single* global script slot — defer the K-slot rack, registry,
MCP and GUI until usage proves them out. The original design that still holds:
fixed `[f32; 32]` indexed array (not a string-keyed map) bound at compile time so
the voice read is `global_mod_bus[idx]` (zero string compares in the voice loop);
index wrapped in a `GlobalBusSlot` newtype; bus threaded via
`ProcessContext.global_mod_bus: Option<&'a [f32]>` with the global rack writing
*before* voices read (write fully precedes reads → no aliasing); a resolved bus
reference modelled as an enum in the bind layer (`Builtin(Context)` vs
`UserSlot(GlobalBusSlot)`); dangling `glob.<name>` reads `0` (disable-and-keep).

---

## Phase 4 (B, later stage) — Audio-rate DSP module

Self-contained, opt-in, with a budget guard. Builds directly on Phases 0/2.

- **VM model: scalar warm-frame `eval_block()` as the baseline** — sample loop on
  the outside, ops inside, keeping stack/locals/regs warm across the loop (no
  per-sample setup) + a **source split: per-block constant vs per-sample** (macros
  / slow params resolved once per block; only audio-in + `phasor`/state tick per
  sample). This is the lever that halves–thirds the audio-rate cost.
  - **Why not a vectorized (op-over-the-block) VM as the primary model:** it
    conflicts with per-sample feedback (next bullet). A vectorized VM runs each op
    across the whole block before the next op, so a `StoreState` writes the entire
    block before a later `LoadState` reads it — the `y[n] = f(x[n], y[n-1])`
    recurrence is broken. Vectorization only works for **feed-forward** graphs
    (waveshaper, feedback-free FM, mixing). It also needs `block_size`-wide stack
    slots (~64 KB value stack) in pre-allocated per-voice scratch, ~1 MB across
    16 voices — not free. **Decision:** scalar warm-frame baseline (supports
    everything incl. feedback); add a vectorized fast-path *only* for the
    feed-forward subset later, benchmark-gated, never as the only VM.
- **`Op::LoadState(u16)` / `Op::StoreState(u16)`** — push/pop an arbitrary state
  cell, so users can write custom IIR (biquad, allpass, feedback FM) as plain
  difference equations. Requires: (a) the scalar VM above; (b) a small **grammar
  addition to declare/reserve state** (e.g. `state s = 0`) so the compiler maps
  named state to cell indices without colliding with `lag`/`phasor` allocation;
  (c) a `docs/yams.md` stability note — layer-2 `finite_or_zero` stops NaN but not
  amplitude runaway in an unstable loop (user's responsibility).
- **`first_sample` context var (audio-rate one-shot init).** At control rate
  `gate_on` fires for the single note-on eval, but at audio rate one eval runs
  *per sample*, so `gate_on` (a per-block flag = `note_on_block`) would read `1`
  for the **whole first block** (e.g. 64 samples). A DSP script using it to
  reset/seed feedback state would re-init 64× → clicks / disabled feedback. Add
  `first_sample` (a.k.a. `sample_on`), `1` only at sample index 0 of the note,
  `0` after. (`edge(gate)` is the in-language equivalent, but a context var is
  clearer and matches `gate_on`'s intent.)
- **Audio-input ports** as per-sample sources (`in`, `in-l`, `in-r`).
- **Multi-out grammar** (`out.left = …`, `out.right = …`) — the one real
  grammar/compiler change, required for stereo.
- **`ModuleType::AudioScript`** with audio in/out ports, per-sample eval.
- **CPU meter / budget guard** + opt-in (perf model: heavy stereo script × full
  polyphony ≈ half a core per instance).
- **Documented capability/limit:** pointwise signal function + 1-pole states
  (`lag`) **plus** user-addressable state cells (`LoadState`/`StoreState`) → custom
  IIR/biquad/short feedback now in scope. Still no large buffer memory →
  no long delay/reverb/FFT/convolution. Good for
  waveshaper/folder/bitcrusher/ring-mod/FM/custom-osc/custom-filter.

**Files:** `eval.rs` (scalar `eval_block` + source kinds + state opcodes),
`bytecode.rs` (multi-out, `LoadState`/`StoreState`), `parser.rs`/`compile.rs`/`fmt.rs`
(`out.x` grammar + `state` declarations), new `synth_modules/src/audio_script.rs`,
`benches/` (criterion: scalar vs vectorized on the feed-forward subset), GUI CPU
indicator.
**Test:** `out = tanh(in * drive)` vs a reference waveshaper; a scripted biquad vs
a native biquad (validates `LoadState`/`StoreState` feedback); criterion bench
confirms the audio-rate cost; null test (passthrough) is bit-exact.

---

## Cross-cutting: test & verification

- **Per phase:** `cargo fmt --check && cargo build && cargo clippy --all-targets
  && cargo test` green (per CLAUDE.md).
- **RT safety:** keep the allocation-free / lock-free invariant in every new eval
  pass; no `unwrap` in `process()`.
- **Phase B:** criterion benchmarks are mandatory — the perf model is a
  hypothesis until measured.
- **Eyeball in-app** for every GUI-bearing phase (Phases 2–4) per the `verify`
  skill.

## Resolved by the RT/DSP review (folded in above)

- **PRNG decorrelation** → `PolyModule::set_voice_index` (Phase 0).
- **`Arc<BoundScript>` drop on the audio thread** → pre-existing hazard in the
  shipped Mod Matrix; **already fixed on `main`** via a new `script_trash`
  deferred-drop channel (`set_mod_script` returns the replaced `Arc`).
- **Transport sources** → already in `ProcessContext` (`position_beats`,
  `tempo`); newtypes at the resolve boundary, `f32` in the VM.
- **Named-source resolution** → pre-bound to fixed accessors/indices at bind
  time; no string compares in the voice loop (Phase 1).
- **Script output** → fill the *entire* buffer, not `buffer[0]` (Phase 2).
- **K = 8** slots/outputs (Phase 2).
- ~~**Global bus** → fixed `[f32; 32]`, name→index at compile time, threaded via
  `ProcessContext.global_mod_bus`~~ (Phase 3 — **dropped**; minimal-slice design
  retained in the Phase 3 note).
- **Audio-rate VM** → scalar warm-frame baseline (vectorize only the
  feed-forward subset, benchmark-gated) + `LoadState`/`StoreState` + a `state`
  declaration for custom IIR (Phase 4).
- **`first_sample` context var** → audio-rate one-shot init, distinct from the
  per-block `gate_on` (Phase 4).

## Resolved by the second review round

- **Source model → address-based.** Keeps the `src lfo = lfo-1.out` workflow, no
  virtual cables in the GUI; the ~1-block latency (~1.3 ms at 48 kHz / 64) is
  negligible for modulation. Port-based / zero-latency feedback is only needed at
  audio rate, which is the separate `AudioScript` module (Phase 4) — the
  control-rate `Script` module stays address-based.
- **Vectorized fast-path → deferred.** Write the scalar `eval_block` first and
  make it support `LoadState`/`StoreState`; only revisit vectorization if
  criterion shows instruction dispatch is the bottleneck on the feed-forward
  subset.

## Decisions still open before coding

*(None — both prior open decisions resolved above.)*
