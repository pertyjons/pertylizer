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

## Status (verified against `main`, 2026-06-30)

- **Phase 0 — DONE.** `ScriptHost` (`synth_core/src/script/host.rs`) is built and
  wired; `RegisterFile` lives in the host inside each module (no `script_regs` on
  `Voice`); two-step resolve→`eval_script_slot`; `PolyModule::set_voice_index`;
  `set_script` returns the replaced `Arc` (`#[must_use]`); `script_trash`
  deferred-drop ring. (One small bullet — zeroing a slot's state on a *live* script
  swap — is described below and may still be open; verify before relying on it.)
- **Phase 1 — DONE.** Transport sources `beat`/`bar_phase`/`tempo`/`playing` exist
  (`synth_script/src/symbols.rs`, `synth_core/src/script/bound.rs`).
- **Phase 2 — DONE.** The standalone control-rate **`ModuleType::Script`** module
  exists (`synth_modules/src/script_module.rs`): K=8 slots `out1`..`out8`, embedded
  `ScriptHost`, whole-buffer fill, chainable (`src x = scr-1.out1`). It is
  control-rate — one value per block, broadcast across the buffer.
- **Phase 3 (C) — dropped** (see note below).
- **Phase 4 — IN PROGRESS** (branch `feat/yams-audio-script`), built in logical
  sub-steps:
  - **Step 1 — VM foundation: DONE.** Scalar warm-frame
    `CompiledScript::eval_block()` (`synth_core/src/script/eval.rs`): the
    op-dispatch loop is now a shared `run()` behind both per-block `eval` and
    per-sample `eval_block`; `Stack::clear()` resets `sp` only; `Op::LoadState` /
    `Op::StoreState` (author-addressable IIR state, layer-2 NaN-sanitized on
    store) and `Op::StoreAudioOut(chan)` (multi-out) added to `bytecode.rs`;
    `AudioBindings` carries the per-block-constant vs per-sample source split
    (audio-in + `first_sample`); `EvalContext::audio()`. 6 unit tests (passthrough
    bit-exact, gain, scripted one-pole via Load/StoreState, stereo multi-out,
    `first_sample` one-shot, NaN-in-feedback clamp).
  - **Step 2a — Compiler: `state` cells + `tanh`: DONE.** `state s = 0`
    declaration (header) + `s = expr` assignment (ordered body statement) compile
    to `Op::LoadState`/`StoreState`, routed through `alloc_state` (shared
    `MAX_STATE` cap). The program body is now an ordered `Vec<BodyStmt>` (locals +
    assignments) so straight-line order gives correct IIR semantics. `tanh`
    builtin added. Non-zero state init is rejected with guidance toward
    `first_sample`. Works at control rate too (custom one-pole etc.). 8 new tests
    across lexer/parser/compile/fmt. (`synth_script` + `synth_core/bytecode.rs`.)
  - **Step 2b — Compiler: audio-rate grammar: DONE.** `CompileOptions.audio_rate`
    flag gates the audio-only grammar (compile error in a control-rate script):
    audio-in sources `in`/`in_l`/`in_r` (`in` = left; underscores, since `-`
    lexes as Minus), the `first_sample` one-shot, and `out.left`/`out.right`
    multi-out (`Op::StoreAudioOut`, with mono/channel mix + duplicate-channel
    validation). New runtime types `ScriptContext::FirstSample`,
    `ScriptInput::AudioIn(AudioInputChannel)`; voice.rs resolves both to 0.0
    block-constant placeholders (the audio module injects per-sample). 5 new
    tests incl. a compile→`eval_block` `tanh(in*4)` waveshaper round-trip.
  - **Step 3 — `AudioScript` module + engine wiring: DONE.**
    `synth_modules/src/audio_script.rs`: a per-voice `PolyModule` running its YAMS
    program once per sample via `CompiledScript::eval_block` in `process()`. Holds
    a single `Arc<BoundScript>` + one `RegisterFile` (not the 16-slot
    `ScriptHost`); stereo `in_l`/`in_r` → `out_l`/`out_r`; owns per-channel scratch
    (a `HashMap` can't hand out two `&mut` channel buffers). `set_script` computes
    `AudioBindings` + sizes the source buffer; `first_block` drives the
    `first_sample` one-shot (only cleared once a non-empty block ran). New
    `ModuleType::AudioScript`, factory registration, and voice wiring
    (`audio_script_ids`, `prepare_audio_script` resolves block-constant sources
    each block; refreshed in `update_output_cache` too). 7 module tests + a voice
    integration test (per-sample DSP + block-constant `velocity`). **GUI add-module
    palette entry deferred to Step 4** (the module is reachable via MCP/load).
  - **Step 4 — benches + docs + authoring dialect: DONE (headless).** Criterion-
    style `harness=false` bench (`synth_script/benches/yams_audio.rs`) measures the
    audio-rate `eval_block` cost (simple waveshaper ≈3% of a core at 16 voices; a
    heavy 25-op biquad ≈9%) — validating the perf model. `docs/yams.md` documents
    `state` cells, `tanh`, and a full audio-rate section (in/in_l/in_r,
    `first_sample`, multi-out, cost/stability). The authoring/persistence dialect
    is wired: `compile_mod_script(source, audio_rate)` selects control- vs
    audio-rate by the target module type, threaded through `set_mod_script` (live
    author + live project load + MCP) and the offline renderer — so AudioScript
    programs compile, save/load, and render correctly (headless test
    `mod_script_dialect_follows_module_type`). The data-driven "Add module" menu
    already reaches `AudioScript` (it's in `ModuleType::all()`).
  - **Step 5 — GUI editor panel: DONE (headless; needs in-app eyeball).** The
    `AudioScript` module now renders a single-slot grid
    (`draw_audio_script_module_grid`, `patch_editor.rs`) dispatched on the
    `"audio_script"` descriptor (`node.rs`), reusing the shared expression-editor
    popup (`draw_slot_expression_editor`) — the same code field as the control
    Script module. A shared `draw_script_slot_row` helper now backs both the
    Script (8-slot) and AudioScript (1-slot) grids. `ScriptEditorState` carries an
    `audio_rate` flag so the popup's live compile, window title, hint, and the
    YAMS reference/help panel match the dialect
    `session::compile_mod_script(.., audio_rate = true)` actually installs (and
    the control-script cycle-warning is suppressed for the intentional per-sample
    IIR feedback). The help panel branches its execution-model and sources
    sections to the per-sample dialect (`in`/`in_l`/`in_r`, `first_sample`,
    `state` cells, `out.left`/`out.right`) — fixed in the final xhigh review,
    which two independent finders flagged as the one real defect. The program is
    exposed by the engine as slot "1" and mirrored into `slot_scripts[0]` by
    `sync_module_scripts`, so the install/preview paths are identical to Script.
    Workspace green.
  - **Step 6 — live CPU-cost indicator: DONE (headless; needs in-app eyeball).**
    The audio-program editor now shows a per-sample instruction count plus a
    rough core-fraction estimate (`popups.rs`, audio-rate only). Because YAMS
    evaluates every branch every sample (eager), the compiled `code().len()` is
    the worst-case per-sample cost by construction, so it is a faithful CPU
    proxy; the percentage is anchored to the `yams_audio` bench (~25 ops ≈ 9% of
    one core at 16 voices / 48 kHz) and labelled an estimate. Tiered colour
    (green ≤24 ops, yellow ≤64, orange beyond) + a tooltip explaining it scales
    with voice count / sample rate and steering authors away from heavy DSP in
    ternary branches. **Deliberately not a live audio-thread measurement:**
    per-module timing is dead infra (`cpu_tracker.rs`) and instrumenting the
    per-sample voice loop would add forbidden syscalls to the RT hot path (only
    the callback-level aggregate `cpu_usage`/`cpu_breakdown` is measured); the
    static worst-case estimate is the RT-safe, honest choice and directly serves
    the external review's "warn against heavy ternary branches" point.
  - **Step 7 — offline-render script replay: DONE (headless; regression-tested).**
    Bug found while building a demo AudioScript instrument over MCP: the patch
    played live but `analyze_note`/`preview` rendered it SILENT. Root cause was a
    per-renderer divergence — the `OfflineNoteSession` note loader
    (`preview.rs::apply_module_state`) replayed parameters + bypass but never
    installed YAMS scripts, so every scripted module (asc / scr / mmx) rendered
    zero through the note path (the arrangement renderer already replayed them).
    Fixed by replaying scripts in the note loader, and the two identical replay
    blocks were de-duplicated into a shared `audio::replay_module_scripts` helper
    (used by both `arrangement_render` and `preview`). New regression test
    `audio_script_program_is_replayed_in_offline_note_render` (preview_integration.rs):
    `out = in` passthrough is silent with no program, audible once installed.
    Note: this also restored control-rate Script/Mod-Matrix scripts in
    `analyze_note`, not just AudioScript. (`render_to_wav` was never affected —
    the arrangement path already replayed scripts; my earlier MCP report
    conflated the two.)
  - **Step 8 — in-app eyeball + bundled example: DONE.** With the app rebuilt to
    this branch (egui inspection on), the GUI was verified live via the egui MCP:
    the AudioScript ƒx editor renders the audio-rate dialect (title "Audio program
    - per-sample DSP", the `out = tanh(in*4)`/stereo hint, the code field, a green
    "compiled" status, and the CPU line "~59 ops/sample · ~21% core @ 16 voices
    (est.)"), and the `asc-1` panel shows the single-slot PROGRAM/dsp/ƒx grid. The
    offline-render fix was live-verified earlier via `analyze_note` (silent before,
    audible after rebuild) on a stereo wavefolder patch. A bundled example project
    `assets/examples/projects/YAMS AudioScript Wavefolder.json` (a stereo
    per-sample wavefolder that also synthesizes its own sub-octave inside the
    script) was saved and round-tripped (build → save → load → render, non-silent,
    stereo). NOTE: the egui a11y tree doesn't expose the patch-canvas popup widgets
    (custom painters + reactive repaint), so the eyeball relied on screenshots, not
    `query_tree`/`click`.
  - **PHASE 4 (B) COMPLETE.** All 8 steps done; branch `feat/yams-audio-script`
    (now ~13 commits, workspace green). NOT yet merged — squash-merge to main per
    CLAUDE.md is the user's call.
- **Future / out of plan:** a single-instance `AudioEffect` flavor of the audio
  script for effect chains / return buses / the **master bus** (the per-voice
  `PolyModule` script cannot live there — effects are a separate trait, one stereo
  instance on summed audio). Cheapest audio-rate target (no polyphony multiplier).

**Net: Phases 0–2 shipped; the live work is Phase 4 (+ the optional master/bus
`AudioEffect` flavor).**

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

## Phase 0 — Foundation: make script execution module-agnostic — DONE (on `main`)

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
- **State hygiene on live swap — clear the slot's `RegisterFile` state when a
  script is (re)installed.** `set_slot`/`set_script` today `mem::replace` the
  `Arc<BoundScript>` but leave `regs[slot].state` untouched
  (`host.rs`, `eval.rs`). State-cell *indices* are positional and bytecode-defined:
  cell 0 may be a `lag` memory in the old script and a feedback accumulator in the
  new one. Swapping on a **sounding** voice (GUI/MCP live edit) thus feeds foreign
  state into the new script — `finite_or_zero` blocks NaN/Inf but not a large finite
  carryover → DC offset / click at control rate, potential amplitude spike in a
  Phase 4 audio-rate feedback loop. Fix: have `set_slot` also zero that slot's
  state cells (reuse the same `reset_state_only` primitive introduced for free-run
  mode below; leave the PRNG seed so voice decorrelation survives). A swap is
  already a discontinuity, so starting the new script from known-zero state is more
  predictable than inheriting the old layout.
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

## Phase 1 (E) — New sources: transport, tempo, more params — DONE (on `main`)

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

## Phase 2 (A) — The Script module — DONE (on `main`)

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
  per-sample setup — reset the value stack per sample via a new `Stack::clear()`
  that sets `sp = 0`, *never* `Stack::new()`, which `memset`s the 64-float buffer
  every sample; stale slots above `sp` are unread) + a **source split: per-block
  constant vs per-sample** (macros
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
  - **Why scalar interpretation is expected to hold up (perf hypothesis,
    statically verified):** YAMS bytecode is *branchless straight-line* code — no
    jump/branch/loop opcodes exist, and even the `?:` ternary lowers to an eager
    arithmetic-mux `Op::Select` that evaluates *both* arms (comparisons/logic
    push 1.0/0.0). So the interpreter's indirect-dispatch sequence is identical
    on every sample → the BTB memorizes it (near-zero mispredicts) and the
    ≤`MAX_INSTRUCTIONS` (256) program fits L1-I. This is the reason the scalar
    baseline can carry everything and the vectorized path stays deferred until a
    bench proves dispatch is the bottleneck. **Op-budget caveat:** eager `Select`
    means a ternary always pays for *both* arms every sample — `cond ? big : big`
    costs the sum, not the taken branch. (Still a hypothesis until the Phase 4
    criterion bench.)
- **`Op::LoadState(u16)` / `Op::StoreState(u16)`** — push/pop an arbitrary state
  cell, so users can write custom IIR (biquad, allpass, feedback FM) as plain
  difference equations. Requires: (a) the scalar VM above; (b) a small **grammar
  addition to declare/reserve state** (e.g. `state s = 0`) so the compiler maps
  named state to cell indices without colliding with `lag`/`phasor` allocation —
  **route these through the existing `compile.rs::alloc_state`** so they inherit
  its `next_state > MAX_STATE` check (already emits the `"too much state (max N
  cells)"` *compile error*, tested) instead of risking a silent index truncation
  on the audio thread; note `MAX_STATE` is currently **16** cells — fine for a
  biquad/allpass, but may need raising if an audio-rate script chains several
  filters, so size it deliberately when Phase 4 lands;
  the *surface syntax* for a declared state cell is **decided: assignment, not
  side-effecting builtins** — read a bare `s` (compiles to `Op::LoadState`) and
  write `s = expr` (compiles to `Op::StoreState`), mirroring `out = expr`.
  Rationale: every current YAMS builtin (`sin`, `lag`, …) is a *pure* function, so
  a `store_state(s, v)` builtin would be the language's first side-effecting call
  and break the expression-tree model; assignment keeps evaluation and
  state-mutation cleanly separate. Straight-line program order gives correct IIR
  semantics (the `s` read returns the prior sample's stored value, the later
  `s = …` write updates the cell for the next sample);
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
  grammar/compiler change, required for stereo. `out.left`/`out.right` compile to
  `Op::StoreAudioOut(0)`/`(1)`; a bare `out = expr` on a stereo module duplicates
  to both channels (mono-compat default).
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
confirms the audio-rate cost; null test (passthrough) is bit-exact; a NaN/Inf
injected into a feedback loop clamps to zero (`finite_or_zero`) without blowing up
the engine.

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

## Resolved by the third review round (DSP/RT)

A later DSP-focused review re-raised several RT hazards. Cross-checked against the
shipped code, **most are already handled** — recorded here so they aren't relitigated:

- **Denormal protection (FTZ/DAZ) → ALREADY DONE, thread-wide.** `DenormalGuard`
  (`synth_core/src/types/denormal.rs`) sets FTZ+DAZ via MXCSR on x86_64 and is
  instantiated at the top of the cpal audio callback
  (`cpal_backend.rs:319`), with tests asserting the bits set/restore. Because it
  is set at the callback level it covers the **entire** audio thread, including any
  future audio-rate YAMS VM feedback loops (Phase 4) — the "decaying IIR FPU stall"
  concern is already neutralized. `FilterState::flush_denormals()` remains the
  non-x86 fallback. No action.
- **Zipper noise / parameter smoothing → ALREADY DONE.** The `lag` opcode exists
  (`Op::Lag`, `eval.rs`; one-pole in `bytecode.rs`) and is documented in
  `yams.md` (`lag(mod_wheel, 50ms)`). Whole-buffer fill (Phase 2) already removes
  the missing-sample case. No action beyond keeping the docs emphasis.
- **NaN/Inf state poisoning → ALREADY DONE.** `finite_or_zero` sanitizes every
  `state_set` (layer 2) plus the final output (`eval.rs`). `is_finite` compiles to
  a cheap exponent-mask check (esp. with FTZ/DAZ on); no pre-optimization needed.
- **Sample-accurate init (`first_sample`) → OPEN, correctly scoped to Phase 4.**
  The review is right that an audio-rate DSP module needs a sample-accurate init
  pulse, not the block-level `gate_on`, to avoid re-running reset logic per sample
  → clicks. This is exactly the Phase 4 `first_sample` task above; no plan change.
- **PRNG retrigger determinism → ALREADY DONE; free-run is an optional add (below).**
  `ScriptHost`/`RegisterFile` re-seed from `voice_index` on note-on
  (deterministic retrigger, voices decorrelated) — see Phase 0. The review's
  "free-run/random retrigger" variant is a genuine *feature request*, captured next.

### Optional follow-up — free-run PRNG retrigger mode

Today every note-on re-seeds each slot's PRNG from `voice_index`, so a retrigger of
the same voice reproduces an **identical** random sequence (reproducible renders,
consistent transients). Add an opt-in **free-run** policy where note-on zeroes the
state cells but leaves the PRNG *streaming*, so each retrigger draws fresh
randomness (analog-style voice drift). Voices stay decorrelated in both modes (the
per-voice seed is established at allocation via `set_voice_index`, independent of
the retrigger policy).

- **Mechanism (synth_core):** `RegisterFile::reset_state_only()` (zero `state`,
  leave `prng_seed`/`prng_counter` untouched) + a `free_run_rng: bool` policy flag
  on `ScriptHost`; `ScriptHost::note_on` branches on it (`reseed()` vs
  state-only). `set_voice_index` always full-reseeds (initial per-voice
  decorrelation) regardless of the flag.
- **Surface (the actual work):** thread the flag through the host modules
  (`mod_matrix.rs`, `script_module.rs`), then save/load + MCP toggle + a GUI
  control. Default stays **deterministic** (current behavior, byte-identical
  renders).
- **Semantics — paused-and-resume, not absolute-time.** A voice's PRNG only
  advances while the voice is *sounding*; an idle/unallocated voice freezes its
  `prng_counter`, so a later note resumes the stream where it stopped rather than
  reflecting wall-clock elapsed time. This is the right trade-off for "analog
  drift" (predictable, reproducible-per-session) and is what this follow-up
  builds. True absolute-time free-run (drift progresses even while silent) would
  require folding the engine's running block/sample clock into the seed/counter at
  note-on — explicitly **out of scope** for this low-priority feature; record it
  here only as the upgrade path if a patch ever needs it.
- **Priority:** low / nice-to-have. Only build it if a patch actually wants drift —
  native LFOs already cover most free-running modulation.

## Resolved by the fourth review round (DSP/RT round 2)

A second pass over the round-3 updates raised three edge-cases. Verdicts after
cross-checking the code:

- **Live script-swap state poisoning → REAL; folded into Phase 0.** `set_slot`
  replaces the `Arc` but leaves `regs[slot].state`; a swap on a sounding voice
  feeds the old script's positional state cells into the new bytecode. Action: zero
  the slot's state on (re)install (new Phase 0 bullet above, reusing
  `reset_state_only`; PRNG seed kept for decorrelation).
- **Free-run PRNG is paused-and-resume, not absolute-time → ACCEPTED trade-off,
  documented.** The PRNG counter freezes while a voice is idle, so a retrigger
  resumes the stream rather than reflecting wall-clock time. Fine for analog drift;
  absolute-time upgrade path (fold the block/sample clock into the seed) recorded
  in the free-run follow-up but left out of scope. No plan change beyond the note.
- **Phase 4 `state` declarations exceeding `MAX_STATE` → ALREADY handled by the
  allocator.** `compile.rs::alloc_state` already emits a `"too much state (max N
  cells)"` *compile error* when `next_state > MAX_STATE` (tested). Action is only to
  route the new `state s = …` grammar through that same allocator (clarified in the
  Phase 4 bullet) and to size `MAX_STATE` deliberately for audio-rate use.

## Decisions still open before coding

*(None — both prior open decisions resolved above.)*
