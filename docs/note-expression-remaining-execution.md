# Execution plan: remaining note-expression / automation work

Companion to `docs/note-expression-roadmap.md`. Phases **A1, A2, B, C are done**
(shipped through v0.295.0). This is the **commit-sized execution plan** for what
remains, sequenced for a step-by-step loop: each step is one logical commit,
**preceded by a `/code-review`**, the build gate, and `gen_schemas` when the
on-disk schema changes.

## What remains (from the roadmap)

| Track | What | Blocked? |
|---|---|---|
| **F** — A1/A2 deferred cross-cutting follow-ups | param-id interning · stable ModuleId identity · combine ordering · offline-render parity | No — do first |
| **P** — export robustness (Parallel track) | `get_project_schema` MCP tool · file-level load-lint | No — cheap, independent |
| **D** — shared/bus filter w/ automatable cutoff | bus effect chain + shared filter + automation target | **Yes** — needs channel-strip Phase 7 |
| **E** — full Note Expression + MPE (north star) | per-note curves, generic per-note targets, MPE, AWE-spatial, Note Processors, curve UI | **Yes** — needs a UX plan first; optional |

**Loop scope = Track F then Track P.** Both are well-defined and commit-sized.
**The loop must STOP at the Phase D and Phase E gates** (see those sections) —
they need a prerequisite (channel-strip Phase 7) or a design doc (E UX plan) that
is itself a separate planning task, not a code step.

## Loop protocol (every step)

1. Implement exactly the one step.
2. `/code-review` the working tree (effort per the step's note; `high` for the
   audio-thread steps F1/F3/D2/E*). Apply or consciously dismiss each finding.
3. Build gate: `cargo fmt --check && cargo build && cargo clippy --all-targets && cargo test`
   — all zero-warning. **Verify cargo's true exit code**, not a piped stage's
   (a `| rg` pipe masks compile failures — bitten twice in B/C).
4. `cargo run -p pertylizer --bin gen_schemas` **iff** the on-disk schema changed;
   commit the regenerated `schemas/*.json`.
5. Commit with the step's suggested message. Do **not** bundle two steps.
6. Flip the step's checkbox here; flip the roadmap `Status`/glance line only when
   the whole phase/track lands; append a `docs/history.md` line + version bump at
   track close.

## Verified code anchors (confirmed 2026-05-29)

| Concern | Location |
|---|---|
| `AutomationTarget::Module { …, param_id: String }` | `synth_sequencer/src/automation.rs:216-224` |
| Audio-thread `target.clone()` (heap-allocs the `param_id` String) | `synth_engine/src/sequencer_engine.rs:565,672,736,753` |
| String-interning model to copy (`PortName`, u32 `Copy`, lock-free pool) | `synth_core/src/types/interned.rs`, `types/mod.rs:48` |
| Module-param override dispatch (positional `module_type`+`instance`) | `synth_engine/src/synth_engine.rs` (`apply_module_param_override`), `instrument.rs:982` |
| Engine `ModuleId { module_type, instance: u16 }` (positional) | `synth_engine/src/commands.rs:43` |
| GUI lane target picker | `crates/pertylizer/src/gui/sequencer/mod.rs` (~3375) |
| MCP automation-target bridge (`module:<prefix>:<instance>:<param_id>`) | `crates/pertylizer/src/mcp_bridge.rs:5566-5690` |
| Offline render session (no automation today) | `crates/pertylizer/src/audio/arrangement_render.rs` (`OfflineEngineSession::render_range`) |
| `analyze_*` offline-render snapshot bug class (reference fix) | `docs/history.md` 74d18da; memory `project_analyze_offline_render_snapshot_bug` |
| mod-matrix additive offset vs automation absolute override | roadmap §"automation value model"; `graph.rs:454` (`apply_mod_offset`) |
| `get_graph_diagnostics` (P2 builds on it) | `crates/pertylizer/src/mcp_bridge.rs`; memory `project_diagnostics_effect_chain` |
| Channel-strip Phase 7 (Phase D prerequisite, **not started**) | `docs/channel-strip-c-plan.md` §"Phase 7" |

These mirror `docs/TODO.md` §"Phase A1/A2 deferred follow-ups" — keep both in sync.

---

## Track F — A1/A2 deferred cross-cutting follow-ups

Retires the automation debt explicitly deferred from the A1/A2 first cut
(roadmap §"pitfalls & open design questions", the four `- [ ]` items). Order is
deliberate: **F1 first** because it reshapes `AutomationTarget::Module`, which
F2/F3 also touch.

### F1 — RT-safe `param_id` (Arc-interned) ✅ (next commit)
**Why:** `sequencer_engine` clones `AutomationTarget` per tick on the audio
thread; a `Module` target heap-allocated its `param_id: String` per clone — a real
RT-safety violation in shipped code.

- [x] **Chose `ParamId(Arc<str>)` over the roadmap's intern-pool `Copy` handle.**
  Reason found at implementation time: the dispatch matches `param_id` against each
  descriptor's `type_id` **string**, so an intern handle would force a lock-taking
  `as_str()` on the audio thread — trading an alloc for a lock. `Arc<str>` clone is
  an atomic refcount bump (RT-safe: no alloc, no lock), and the dispatch keeps its
  lock-free `&str` compare. (Enabled serde `rc`; `#[serde(transparent)]` +
  `#[schemars(transparent)]` → serializes as a bare string, projects load unchanged.)
- [x] All four `target.clone()` sites are now alloc-free; the `BTreeSet<String>`
  reference index (UI-thread) takes `param_id.as_str().to_owned()`; MCP token
  round-trips via `ParamId::from` / `Display`; all construction sites build via `.into()`.
- [x] Tests: serde round-trip asserts the bare-string on-disk form (back-compat).
  Schema regenerated (`param_id` → a `ParamId` string `$def`, value unchanged).
- **Verify:** `/code-review` **high** — clone-side RT goal achieved + back-compat clean.
- **Commit:** `Automation F1: RT-safe AutomationTarget::Module param_id (Arc<str>)`
- **Deferred (recorded):** the corresponding **drop** of a clone on the audio
  thread frees *iff* the source lane was removed mid-playback (engine's cached
  clone becomes last ref). Strict improvement over the prior `String` (which freed
  on every drop); full fix = route cleared targets through the engine's
  `return_producer` off-thread drop channel. Tracked below.

### F3 — Mod-matrix vs. automation combine ordering ("two controllers") ✅ (next commit)
**Why:** a filter cutoff can be driven by a mod-matrix offset *and* an automation
override at once; the precedence was unspecified.

- [x] Ratified the shipped behavior as the rule: `effective = override.unwrap_or(base)
  + mod_offset` (override replaces base, mod-matrix offset adds on top of the
  override). Verified it's already consistent across implementors (filter +
  amplifier both `override.unwrap_or(base)` then `+ mod_offset`). Documented on the
  **`PolyModule::set_param_override` contract** (central, every module implements it)
  and flipped both roadmap deferred items (value-model + "two controllers" pitfall).
- [x] Added `filter::test_automation_override_then_mod_offset_combine_order` locking
  it: base 1000 + mod (+1 oct) = 2000; override 1500 + same mod = 3000 (on the
  override, not the base); base untouched.
- **Schema:** none (no logic change; doc + test). Review: no-logic-change → no
  regression; rule confirmed accurate across modules.
- **Commit:** `Automation F3: define + test mod-matrix vs automation combine order`

### F4 — Offline-render parity for `analyze_*` ✅ (next commit) — already satisfied
**Why (stale premise):** the pitfall assumed `analyze_*` reads base values offline.

- [x] **Verified by code + test: already correct, no code change needed.**
  `OfflineEngineSession::render_range` runs the **same** engine `process()` as live
  (Play → Seek → process), which advances the `SequencerEngine` (collects automation
  → emits `Parameter` events) and `route_sequencer_events` applies the override. The
  pitfall predated this offline-session design (the existing determinism tests
  already cover Track + Global automation offline — just not the A2 `Module` target).
- [x] Added `module_param_automation_ramps_down` (the missing `AutomationTarget::Module`
  case): an amp-`level` lane ramped 1.0→0.0 makes the offline first half clearly
  louder than the second — proof the module override reaches offline audio. Flipped
  the roadmap pitfall.
- **Schema:** none. Review: test-only; proves existing behavior + regression guard.
- **Commit:** `Automation F4: regression test — Module automation reaches offline render`

### F2 — Stable (non-positional) ModuleId identity
**Why:** `AutomationTarget::Module` identifies its target positionally
(`module_type`+`instance`); reordering/removing same-type modules silently
re-points a lane. The biggest of the four — likely **splits into F2a/F2b**.

- [ ] **F2a — introduce a stable per-module id.** Add a stable, non-positional
  identity to a module instance (a persisted `u32`/uuid assigned at creation that
  survives graph edits), alongside the positional `ModuleId`. Persist it; assign on
  load/migration for existing patches. Dual-resolve: lanes still resolve positionally
  until F2b.
  - **Commit:** `Automation F2a: stable per-module identity (assigned + persisted)`
- [ ] **F2b — migrate `Module` lanes onto the stable id.** Switch
  `AutomationTarget::Module` to key on the stable id (engine `ModuleId`, the seq-side
  `{module_type, instance}` key, the GUI picker, and the MCP token all consume the
  positional convention today — migrate each). One-time migration of existing
  positional lanes on project load.
  - **Commit:** `Automation F2b: migrate Module automation lanes to stable id`
- **Schema:** **additive/migrating** (lane target gains the stable id) — `gen_schemas`
  + a load-migration test for pre-F2 projects. `/code-review` **high** each.

### F-close
- [ ] Tick the four roadmap "pitfalls" `- [ ]` items + the `docs/TODO.md` follow-ups;
  `docs/history.md` line; version bump.
- **Commit:** `Automation deferred follow-ups: history + status`

---

## Track P — export robustness (Parallel track)

Independent of everything else; cheap; lands any time. Good loop steps after F.

### P1 — `get_project_schema` MCP tool
- [ ] Add an MCP tool returning the authoritative on-disk `.pertyproj` schema
  (the generated `schemas/project.schema.json`) + a build version string. Fixes
  the introspection-vs-on-disk encoding drift (e.g. `osc.Waveform` reported
  numerically `sawtooth = 2.0` while on-disk is the string `"sawtooth"`); enables a
  CI diff that fires when the format changes.
- [ ] Round-trip test + `README_MCP` entry.
- **Schema:** none (MCP tool param, not persisted). `/code-review` medium.
- **Commit:** `MCP P1: get_project_schema tool (authoritative on-disk schema + version)`

### P2 — file-level load-lint
- [ ] Surface `get_graph_diagnostics` as a single load-lint pass returning
  *warnings* (unconnected ports, silent voices, out-of-range derived values), not
  just schema validation — per-instrument across the whole project. Reuse the
  existing diagnostics scope (see memory `project_diagnostics_effect_chain`).
- [ ] Test + `README_MCP` entry.
- **Schema:** none. `/code-review` medium.
- **Commit:** `MCP P2: file-level load-lint (project-wide get_graph_diagnostics warnings)`

### P-close
- [ ] Flip roadmap **Parallel track** `Status` + glance; `docs/history.md`; version bump.
- **Commit:** `Export robustness: history + status`

---

## Phase D — shared / bus filter with automatable cutoff  ⛔ GATED

**STOP — prerequisite not met.** Phase D rides on **channel-strip Phase 7**
(sends/returns + bus effect chain), which is **not started**
(`docs/channel-strip-c-plan.md` §"Phase 7"). The bus stage today is fader-only
(`synth_engine.rs:2515-2555`). Do not start D in the loop until Phase 7 lands.

The roadmap also notes D is **largely covered by A2** (per-instrument filter
cutoff is already automatable), so its audible payoff is small — sequence it only
if a tune genuinely needs a *shared* SID-style global-filter sweep.

When unblocked (sketch, to be re-planned against Phase 7's bus API):
- [ ] **D1** — extend the bus stage to a bus effect chain (this *is* channel-strip
  Phase 7 — sends/returns).
- [ ] **D2** — let multiple instruments route into a shared bus carrying a filter *(audio thread)*.
- [ ] **D3** — expose the bus filter cutoff as an `AutomationTarget` (reuses A2 +
  the F1/F2 target machinery) → exact shared sweeps.
- [ ] **D-close** — roadmap status + history + version.

---

## Phase E — full Note Expression + MPE (north star)  ⛔ GATED

**STOP — needs a UX plan first.** The piano-roll per-note **curve editor** is the
real cost center; the roadmap explicitly says E "requires its own UX plan before
start" and "stop before it if the UI cost outweighs the richness." Do not start E
in the loop until **E0** (a design doc) exists and is approved.

E reuses everything below it: A2's `(module_id, param)` addressing (via F1/F2's
target), the C expression block (the curve editor *extends* it), and the additive
override path. Sketch sequence (to be expanded in E0):

- [ ] **E0 — UX/design plan** for the per-note curve editor (a doc, not code): data
  model for hand-drawn curves, interaction, how it extends the C `NoteExpression`
  block, performance budget. **This is the gate.**
- [ ] **E1** — per-note curve data model on `Note` (curve points; `Copy`/alloc-free
  or arena'd off the audio thread). Additive serde. Schema regen.
- [ ] **E2** — generic per-note targets: reuse the A2/F-track `(module_id, param)`
  addressing so a curve reaches arbitrary module params *per voice* (the brief's
  "pivotal question"; absorbs Problems 1b/1d/3 into expression) *(audio thread)*.
- [ ] **E3** — engine playback of per-note curves (additive offset path, RT-safe) *(audio thread)*.
- [ ] **E4** — MPE / MIDI 2.0 input mapping onto the primitive-1 dimensions (bend,
  pressure, timbre/slide, velocity, release-velocity) — the set C already defaults to.
- [ ] **E5** — per-note **spatial via AWE** (primitive 1 with an AWE room param as
  the target) — the unique differentiator.
- [ ] **E6** — Note Processors layer (taxonomy primitive 4): arpeggiator, trill/
  mordent/turn (two-note arp), flam/drag/ruff/roll, grace note, strum, chord,
  scale-quantize, humanize. Each *expands* into primitives 1–3 or extra notes.
- [ ] **E7** — voice-allocator polish (unison, voice stealing — partially present).
- [ ] **E8** — piano-roll per-note curve editor UI (the gating UI investment).
- [ ] **E-close** — roadmap status + history + version.

---

## Recommended loop order

1. **F1** (RT-safety — fixes a real audio-thread alloc in shipped code).
2. **F3** (small; ratifies the combine rule the rest assume).
3. **F4** (correctness — stops `analyze_*` from lying).
4. **F2a → F2b** (stable identity; the largest of the four).
5. **F-close.**
6. **P1 → P2 → P-close** (independent export wins).
7. **STOP.** D needs channel-strip Phase 7; E needs the E0 UX plan. Re-plan each
   when its gate clears — don't autostart them in the loop.

## Deferred / out-of-scope (record, don't silently drop)

- Per-note glide & vibrato are **dropped on a stolen voice** (`steal_for`/
  `pending_note` carry no `NoteTrigger`) — B3/C4 deferral; fix by threading the
  trigger through the steal fade-out. Independent of the tracks above.
- The inspector multi-edit **flattens per-note variation** of the edited field
  (matches the velocity inspector) — intentional; revisit only with a relative-delta
  UX.
- MCP `update_notes` cannot edit the expression block (only add/replace + GUI) —
  add an expression field to `NoteUpdateInput` if needed.
- **Audio-thread drop of automation state.** `last_automation_values` and the
  emitted-event buffer hold `ParamId(Arc<str>)` clones cleared on the audio thread;
  if the source lane was removed mid-playback the clone can be the last ref and
  free on the RT thread (F1 review finding). Strictly better than the pre-F1
  `String` (always freed); full fix = route the cleared targets through the
  engine's `return_producer` off-thread drop channel (as modules/instruments
  already are). Independent of the tracks above.
