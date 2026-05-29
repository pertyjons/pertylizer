# Plan: Note Expression & sequencer-driven module automation

A staged roadmap toward per-note expression (the north star for Pertylizer as a
serious synth) that *also* retires the worst faithfulness gaps reported by the
`sid-analyzer` export side. Origin brief:
`/home/per/github/sid-analyzer/docs/pertylizer-schema-investigation.md`.

The ordering is deliberate: each phase ships on its own, and every phase reuses
the groundwork of the one before. The shared bedrock is a stable
`(module_id, param)` addressing scheme — which already exists via
`SetModuleParameter` / `Param` (`commands.rs:364`, applied in
`handle_set_module_param`, `synth_engine.rs:~1407`) — so the cheap phases lay the
foundation the expensive polyphonic layer later rides on.

**Damage axis** (from the brief's prioritization lens): *broken* = a listener
hears something malfunctioning; *blander* = paler but not wrong. Fix artifacts
first, richness second.

## Status at a glance

- [x] **Phase A1** — Wire the 6 already-GUI-exposed instrument macros (bugfix; no schema change)
- [x] **Phase A2** — `AutomationTarget::Module { instrument, module_id, param }` (generic, additive)
- [x] **Phase B** — Per-note legato/tie + glide fields, driving the existing allocator machinery
- [x] **Phase C** — Per-note vibrato depth + a small per-note expression block (note expression in miniature)
- [ ] **Phase D** — Shared/bus filter with automatable cutoff (rides on channel-strip Phase 7)
- [ ] **Phase E** — Full Note Expression: per-note custom curves + generic per-note targets + MPE
- [ ] **Parallel track** — `get_project_schema` MCP tool + file-level load-lint (export robustness)

Mark each `- [ ]` as `- [x]` and flip the per-phase **Status** line when it lands.
Keep `docs/history.md` updated against each ship date.

> **Phases B & C have a commit-sized execution plan:**
> `docs/note-expression-b-c-execution.md` (one logical commit per step,
> `/code-review` + build gate before each). Use it when working B/C in a loop.
>
> **The remaining work (the A1/A2 deferred follow-ups, Phase D, Phase E, and the
> Parallel track) has its own commit-sized plan:**
> `docs/note-expression-remaining-execution.md`. Loop-able now: the deferred
> follow-ups (Track F) + export robustness (Track P); Phase D and Phase E are
> gated (channel-strip Phase 7 / a UX plan) and the loop stops at those gates.

## Context — verified engine state (2026-05-28)

The export side's view of Pertylizer is stale; the points below were confirmed
against the live code. The headline: **more is already built than the brief
assumes** — the missing piece is the *sequencer ↔ engine expression bridge* and
the *generic addressing*, not the DSP.

- **Automation targets are 3 variants only.** `AutomationTarget::{Instrument,
  Track, Global}` (`synth_sequencer/src/automation.rs:189-201`). No `Module`
  variant exists.
- **6 of the 8 instrument macros are dead — and GUI/MCP-exposed.**
  `AutoInstrumentParam` has Volume, Pan, FilterCutoff, FilterResonance, Attack,
  Decay, Sustain, Release (`automation.rs:224-234`). At playback only Volume/Pan
  apply; the rest are `_ => {}` (`synth_engine.rs:2660`,
  `// FilterCutoff etc. requires module routing (future)`). Yet the GUI lane
  picker offers **all** of them (`sequencer/mod.rs:3433`,
  `for param in AutoInstrumentParam::ALL`) and the MCP bridge parses all 6
  (`mcp_bridge.rs:5269-5274`). So a user can draw FilterCutoff/ADSR automation
  today, save it, and it does nothing — a silent no-op in the UI.
- **The `Note` is gate+pitch only.** 6 fields (`id, start, duration, pitch,
  velocity, track`) — `synth_sequencer/src/note.rs:13-26`. No legato/tie, glide,
  vibrato, or per-note overrides.
- **But the engine already has the pitch-expression primitives.** Voice allocator
  has `AllocationMode::{Polyphonic, Mono, Legato, Unison}` (`voice_allocator.rs`),
  Legato changes pitch *without re-gating* (`voice_allocator.rs:248`), and
  `GlideState` does exponential portamento (`voice.rs:214`). These are
  per-instrument/allocator config, **not per-note and not driven by the
  sequencer** — that gap is Phase B.
- **Modulation already reaches pitch — via the mod matrix.** Standard
  `oscillator.rs` has no `freq_cv` port (FM/PM/PWM only,
  `oscillator.rs:403-438`), *but* the mod matrix has `ModDestination::OscPitch`
  (semitones) — an LFO→pitch vibrato works at patch level today
  (`synth_core/src/params/mod_matrix.rs`). The mod matrix is per-voice with a
  **fixed** destination set, not generic, and not sequencer-driven.
- **No shared filter.** The channel bus (channel-strip Phases 1–6) is
  **fader-only**, keyed per `InstrumentId` (`synth_engine.rs:2515-2555`); filters
  live per-voice inside each instrument's graph. Bus effects = channel-strip
  Phase 7 (`docs/channel-strip-c-plan.md`, not started).

---

## Cross-cutting — module references (visibility + integrity)
**Applies to:** A2, D, E (everything that holds a `(module_id, param)` reference)
· **Schema:** none (derived state)

The moment an automation lane points at a module param, that lane becomes a
*dependant* of the module. Two things must follow from a single "who references
this module?" query, reused everywhere:

- [x] **Visibility in the Rack view.** Done: a `ri::PULSE_FILL` *automated* badge on
  the module's panel header (`patch_editor.rs` header row), driven per frame from the
  reference index. (Per-param control badge not yet — module-level only for the first cut.)
- [x] **Deletion guard.** Done (block-with-warning): `egui_backend` blocks removing a
  module that an automation lane targets and surfaces a `dialog_state` status toast;
  the lane is preserved. (The orphan-flag path and *rename* guard are deferred; note
  that the dispatch already no-ops safely on an absent module, so an out-of-GUI orphan
  is harmless at playback.)
- [x] **One reference index.** Done: `Song::automated_module_params()` (and the
  single-lookup `is_module_automated`) builds `module → [param_ids]` once,
  sequencer-side, so the Rack badge and delete guard share one source of truth.
  Phase D / E register through the same index.

This is *derived* state — no on-disk schema change — but it is the difference
between "generic automation exists" and "generic automation is safe to live in".

---

## Cross-cutting — automation value model (base vs. override)
**Applies to:** A1, A2, D, E (everything that drives a module param from a lane)
· **Schema:** none (runtime layering)

The base param value — set when a patch is created or loaded, or edited by hand on
the knob — must survive automation **untouched**. Automation is a *transient
override*, not a write to the stored value. The engine already proves this split
exists; the danger is that the phase tasks above currently point at the wrong half
of it.

- **Destructive path — do NOT reuse for automation.** `handle_set_module_param`
  → `Graph::set_param` (`graph.rs:332`, `synth_engine.rs:1729`) overwrites the
  module's stored param in both the voice-graph template *and* every live voice.
  If A1/A2 dispatch through this (as their tasks literally say today), the value
  **latches** at the last automation sample when playback stops — there is no
  snapshot/restore anywhere — and a later save persists that latched value *over
  the patch base*, corrupting the patch permanently.
- **Non-destructive path — the model to follow.** The mod matrix never writes the
  base: it computes an offset and `Graph::apply_mod_offset` → `set_mod_offset`
  (`graph.rs:454`, `voice.rs:728`) adds it on top per block, reset each cycle, base
  untouched. But it only covers a **fixed** destination set (OscPitch/FilterCutoff/…),
  the same fixed set noted in Context.
- [x] **Generalize the base+override split to any `(module_id, param)`.** Done:
  `PolyModule::set_param_override(Param)` / `clear_param_overrides()` (default no-op),
  per-module `Option<T>` override storage read as `override.unwrap_or(base)` in
  `process()`, fanned out via `Graph`/`Voice`/`Instrument`, cleared on transport stop
  (`handle_all_notes_off`). The base param is never mutated.
- [x] **Combine rule per target (for automation).** Decided: automation is **absolute**
  (the override *replaces* the base while active); mod-matrix stays an additive offset
  applied on top. Before the first point / in gaps / on stop → the override is cleared
  and the param reverts to base. **Ordering when both drive one param (F3, resolved):**
  the effective value is `(override.unwrap_or(base)) + mod_offset` — override replaces
  base, then the mod-matrix offset adds on top of the override. Documented on
  `PolyModule::set_param_override` and locked by a filter test.
- [x] **Save semantics.** Holds by construction: overrides never touch the stored base,
  so saving always writes the base (knob) value even mid-playback. (Read-mode knob
  *display* of the live automated value is a GUI nicety, not yet implemented.)

---

## Cross-cutting — pitfalls & open design questions
**Applies to:** A1, A2, D, E · these are decisions that must be made *before* the
generic param path ships, not after. Each is verified against current code.

- [ ] **`ModuleId` is positional, not a stable identity.** _DEFERRED (A1/A2 first cut)._
  It is `{ module_type, instance: u16 }` (`commands.rs:43`). A1 and A2 deliberately
  use this positional identity ("first module of that type" for A1; `module_type`+
  `instance` for A2), so a lane silently re-points if same-type modules are
  added/removed/reordered. Accepted for the first cut; a stable per-module identity
  (or a deterministic re-resolution rule) is future work that A2's `AutomationTarget::Module`
  would migrate to.
- [x] **Discrete / enum params can't be smoothly automated.** Done: the automatable
  allowlist (`ParameterDescriptor::is_automatable()` = `modulatable && choices.is_none()`)
  excludes `choice`/enum params (`FilterMode`, `Waveform`, …); GUI/MCP filter on it.
- [x] **Not every param is RT-safe to automate.** Done: same allowlist excludes
  structural/sizing params; the flag's home is the descriptor (`modulatable`, now also
  set `false` on `unison`/`steps`/`pulses`/`rotation`/`length`).
- [x] **Control-rate stepping = zipper noise.** Done (cutoff/volume): per-block linear
  ramp of the effective override value in the Amplifier (`level`) and Filter (`cutoff`).
  (Resonance/pan and other params left un-ramped for the first cut.)
- [x] **Per-voice fan-out + mid-note seeding.** Done: `Instrument::apply_param_override`
  fans the override to the template `voice_graph` **and** every pooled voice each
  update, so a voice triggered after an update inherits the current value via the
  template. (Sub-block mid-sweep seeding of a voice triggered *between* updates is
  bounded by the automation update rate — acceptable for the first cut.)
- [x] **Two controllers, one param.** _RESOLVED (F3)._ A filter cutoff can
  be driven by a mod-matrix offset *and* an automation override simultaneously. The
  combine order is now defined and documented: `effective = (override.unwrap_or(base))
  + mod_offset` — the automation override replaces the base, then the mod-matrix offset
  adds on top of the override. Stated on the `PolyModule::set_param_override` contract
  and locked by `filter::test_automation_override_then_mod_offset_combine_order`.
- [ ] **Offline render must apply the override identically.** _DEFERRED (A1/A2 first
  cut)._ The override layer currently runs in the live `process()` path; the `analyze_*`
  offline renderers do not yet evaluate automation, so they read base values. Bringing
  the offline path onto the same clock is future work (a known "offline reader sees
  state the live engine never wrote" bug class).
- [ ] **`AutomationTarget::Module` `param_id: String` is cloned on the audio thread.**
  _DEFERRED (A1/A2 first cut)._ In `sequencer_engine` the per-tick automation collection
  and event emission `clone()` the target; for a `Module` target that heap-allocates the
  `param_id` String inside `process()`, violating the RT-safety rule (bounded: control-rate,
  only on changed Module points). The fix is to intern the param id into a `Copy` handle
  (like `PortName` interns port names) so the target is alloc-free to clone — a data-model
  change deferred out of the first cut.

---

## Cross-cutting — expression primitive taxonomy (the field-shape guardrail)
**Applies to:** B, C, E · **Schema:** none (a classification that constrains the others)

The set of musical-expression terms is open-ended (portamento, glissando, vibrato,
tremolo, swell, scoop, fall, doit, trill, mordent, flam, drag, grace note, staccato,
tenuto, ghost, accent, pizz, palm-mute…). Modelling *one term = one `Note` field*
explodes the note model and re-implements the same DSP three times. They collapse to
**four orthogonal primitives**; the rule is **place the primitive, not the term.**

1. **Per-note modulator on a generic target** *(Phase C parametric → Phase E curve)* —
   a curve *or* a parametric LFO (depth/rate/delay/shape) aimed at any
   `(module_id, param)` via A2's addressing, applied as an additive offset on top of
   the base (the mod-matrix `OscPitch` path, generalised). **Vibrato** = LFO→OscPitch;
   **tremolo** = same LFO→Amp; **auto-pan** = →Pan; **brightness / MPE-slide** =
   →Filter cutoff; **swell** = ramp→Amp; **scoop/fall/doit** & **pitch-envelope** =
   short curve→OscPitch. Build the primitive once and vary the target — never three
   separate "vibrato/tremolo/auto-pan" features.
2. **Inter-note transition** *(Phase B)* — how note N connects to N+1: a *relation*
   between notes, not state inside one. **Portamento/glide** = continuous;
   **glissando** = stepped/chromatic; **legato/tie** = no retrigger. All are
   `GlideState` + `AllocationMode::Legato`, parameterised on one axis: **interpolation
   type (continuous vs stepped)** — the only thing separating glide from glissando.
3. **Note-shape scalars** *(cheap, additive to `Note` — belongs in B/C, not E)* —
   `Copy` `f32`/`bool` that shape a single note without a curve: **accent**
   (velocity ×), **staccato/tenuto/marcato** (gate % of duration), **ghost/dead**,
   **probability/ratchet**. RT-safe, alloc-free; these are the *only* terms that
   justify a new `Note` field.
4. **Generators / ornaments (Note Processors)** *(Phase E)* — things that *expand*
   into primitives 1–3 or extra notes, and are therefore not storage: **arpeggio**
   (Model B), **trill/mordent/turn** (a two-note arp — the same generator!),
   **flam/drag/ruff/roll**, **grace note/acciaccatura**, **strum**, **humanize**,
   **chord/scale-quantize**.

Three consequences that should shape the Phase C/E field design:

- **Adopt MPE / MIDI 2.0 as the canonical minimal dimension set** for primitive 1's
  defaults — *bend, pressure, timbre/slide, velocity, release-velocity* — so MPE input
  (Phase E) maps 1:1 onto the expression block with no translation layer.
- **Stepped vs continuous interpolation is a curve/transition *property*, not a type.**
  It is the single axis distinguishing glissando from portamento and trill/arp from
  vibrato. Note the inversion of the **zipper-noise** pitfall above: here the steps are
  *intentional holds* and smoothing would be the bug.
- **Per-note spatial via AWE is a genuine differentiator not yet anywhere in this
  plan.** It is just primitive 1 with an AWE room param as the target — per-note
  position in the simulated room. No other synth has it; it earns a Phase E bullet.

**Guardrail (one sentence):** a new musical term must map to an existing primitive +
target, or it is a generator (primitive 4) — only add a `Note` field for a true
note-shape scalar (primitive 3).

---

## Phase A1 — Wire the 6 GUI-exposed instrument macros (bugfix)
**Status:** ☑ Done (branch `feat/automation-a1-a2`) · **Effort:** S · **Axis:** broken → fix · **Schema:** none

The targets already exist in the enum, the GUI, and MCP; only the playback
dispatch is missing. This is a bugfix of a silent no-op, not a feature.

- [x] In the `Parameter` dispatch (`route_sequencer_events`), replace the `_ => {}`
  for FilterCutoff/FilterResonance/Attack/Decay/Sustain/Release with resolution to
  the instrument's filter/envelope module. Applied via the **override layer**
  (`Instrument::apply_normalized_override`), not the destructive `set_param`, so the
  macro never latches over the patch base.
- [x] Resolution convention for instruments with **multiple** filters: the **first
  module of that type in the graph** (lowest `ModuleId` instance). Documented next to
  the dispatch.
- [x] Denormalize `NormalizedValue 0..1` → param range via the cached descriptor
  range/curve (`ModuleGraph::module_descriptor`, zero-alloc on the audio thread).

## Phase A2 — `AutomationTarget::Module` (generic param automation)
**Status:** ☑ Done (branch `feat/automation-a1-a2`) · **Effort:** M · **Axis:** broken → fix (filter/PWM) · **Schema:** additive

The biggest single faithfulness win: turns the analyzer's already-extracted
per-frame PWM and filter-cutoff contours into exact playback instead of a static
midpoint / fixed-rate LFO guess.

- [x] Added `AutomationTarget::Module { instrument: SeqInstrumentId, module_type:
  ModuleType, instance: u16, param_id: String }` (additive enum variant) in
  `automation.rs`. **Shape differs from `{ module_id, param }`:** `AutomationTarget`
  is a `HashMap` key (must stay `Eq+Hash`) but `Param` holds `f32` (not `Eq`), and
  `ModuleId` lives in `synth_engine` (unreachable from `synth_sequencer`). Module
  identity is therefore positional (`module_type`+`instance`, mirroring `ModuleId`)
  and the param is its stable descriptor `type_id` string.
- [x] Dispatched through the **override** layer (`Instrument::apply_module_param_override`),
  not `Graph::set_param`; the engine rebuilds the concrete `Param` via
  `descriptor.id.with_f32(denormalize(value))`. Base param never mutated.
- [x] GUI: lane target picker (`sequencer/mod.rs`) browses the selected instrument's
  modules + automatable params.
- [x] MCP: `build_automation_target` accepts `module:<prefix>:<instance>:<param_id>`
  and `automation_target_info` emits the same canonical form (round-trips).
- [x] Referential integrity + visibility — see the **Cross-cutting** section. A
  targeted module shows an *automated* badge in the Rack view and is delete-guarded.

## Phase B — Per-note legato/tie + glide
**Status:** ☑ Done (v0.293.0) · **Effort:** S–M · **Axis:** broken → fix (arpeggio, portamento) · **Schema:** additive

Retires the two worst pitch artifacts (machine-gun arpeggio re-gating; portamento
staircases). The machinery exists in the allocator — this is the sequencer-side
data + wiring.

- [ ] Add `Note.legato`/tie flag and `Note.glide` (target pitch or signed
  semitones + glide time + interpolation type) in `note.rs`. The interpolation axis
  (continuous vs stepped) makes **glissando** the *same field* as portamento — see the
  expression primitive taxonomy (primitive 2).
- [ ] Drive the existing `AllocationMode::Legato` + `GlideState` per note from the
  sequencer playback path.
- [ ] Precedence: per-note glide overrides the instrument allocator default.
- [ ] GUI: piano-roll tie/legato toggle + a glide handle between abutting notes.
- [ ] Export payoff: arpeggio replays as tied notes under one held gate; slides
  replay as glide.

## Phase C — Per-note vibrato + expression block
**Status:** ☑ Done (v0.294.0) · **Effort:** S · **Axis:** blander → richer (dead leads) · **Schema:** additive

Vibrato already works at patch level (mod matrix LFO→OscPitch); this makes it
per-note and seeds the expression model.

- [ ] Add a small per-note expression block that scales the existing mod-matrix pitch
  path. Per the **expression primitive taxonomy**, this block is primitive 1
  (parametric modulator: vibrato depth/delay/rate) plus the primitive-3 note-shape
  scalars (accent, gate %, ghost, probability). Glide *time* stays with the transition
  in Phase B (primitive 2), not here.
- [ ] This block **is** note expression in miniature — design its field shape against
  the *full* primitive set so Phase E's hand-drawn curves extend it rather than
  replace it.

## Phase D — Shared / bus filter with automatable cutoff
**Status:** ☐ Not started · **Effort:** L · **Axis:** blander → broken (sweep-centric tunes) · **Schema:** additive · **Depends on:** channel-strip Phase 7

The structurally correct model for SID's single global filter. Largely deferred —
Phase A2 already makes a per-instrument filter cutoff automatable, which removes
the "static filter sounds broken" artifact for most tunes without this.

- [ ] Extend the bus stage (`synth_engine.rs:2515-2555`) from fader-only to a bus
  effect chain (this is channel-strip Phase 7 — sends/returns).
- [ ] Let multiple instruments route into a shared bus that carries a filter.
- [ ] Combined with A2, expose the bus filter cutoff as an automation target →
  exact shared SID-style sweeps.

## Phase E — Full Note Expression + MPE (north star)
**Status:** ☐ Not started · **Effort:** L · **Axis:** blander → richer (polyphonic) · **Schema:** additive

Build the full layered system only if/when the polyphonic richness justifies the
**piano-roll per-note curve UI** — the real cost center. Requires its own UX plan
before start.

- [ ] Per-note custom expression curves (pitch bend / brightness / pressure
  *inside* the note).
- [ ] Generic per-note targets: reuse Phase A2's `(module_id, param)` addressing so
  curves can reach arbitrary module params per voice (this is the brief's "pivotal
  question" — answering yes absorbs Problems 1b/1d/3 into expression). This is the
  taxonomy's primitive 1 with a hand-drawn curve as the source.
- [ ] Per-note **spatial via AWE**: primitive 1 with an AWE room param as the target —
  per-note position in the simulated room. Unique differentiator; no equivalent in
  other synths.
- [ ] MPE / MIDI 2.0 input mapping — drive the primitive-1 dimensions (bend, pressure,
  timbre/slide, velocity, release-velocity) directly; this is the canonical minimal
  set the Phase C expression block should already default to (see taxonomy).
- [ ] Note Processors layer = the taxonomy's **primitive 4** (generators that expand
  into primitives 1–3 or extra notes): arpeggiator, **trill/mordent/turn** (a two-note
  arp), **flam/drag/ruff/roll**, **grace note**, **strum**, chord, scale-quantize,
  humanize — optional, for live composition.
- [ ] Voice allocator polish (unison, voice stealing — partially present).
- [ ] Piano-roll per-note curve editor (the gating UI investment).

## Parallel track — export robustness (independent, cheap)
**Status:** ☐ Not started · **Effort:** S · **Axis:** neither (tooling)

- [ ] `get_project_schema` MCP tool returning the authoritative on-disk
  `.pertyproj` schema + a build version string. Fixes the encoding drift where
  introspection reports `osc.Waveform` numerically (`sawtooth = 2.0`) while on-disk
  is string-only (`"sawtooth"`); enables a CI diff that fires when the format
  changes.
- [ ] File-level load-lint: surface `get_graph_diagnostics` as a single pass
  returning *warnings* (unconnected ports, silent voices, out-of-range derived
  values), not just schema validation.

---

## Build order rationale

- **A1 first** — pure bugfix, no schema change, removes a silent-lie UI surface.
- **A2** — lays the generic addressing bedrock that B, C, and E all reuse; turns
  PWM/filter approximations into exact playback.
- **A1 + A2 + B** retire every *broken* artifact (filter sweep, PWM, arpeggio,
  portamento).
- **C** is a cheap richness win and the seed of the expression model.
- **D** waits on an already-planned phase (channel-strip Phase 7) and is mostly
  covered by A2 for audible damage.
- **E** is the expensive polyphonic top; stop before it if the UI cost outweighs
  the richness for the product.
- **Parallel track** is independent of all the above and can land any time.

## Critical files

| Concern | File |
|---|---|
| Automation enum + lanes | `crates/synth_sequencer/src/automation.rs` |
| Note model | `crates/synth_sequencer/src/note.rs` |
| Param dispatch / apply | `crates/synth_engine/src/synth_engine.rs` (~2647 route, ~1720 set_module_param) |
| Base vs. override (value model) | `crates/synth_engine/src/graph.rs:332` (`set_param`, destructive) vs. `graph.rs:454` (`apply_mod_offset`) + `voice.rs:728` (`set_mod_offset`) |
| Voice allocation / glide / legato | `crates/synth_engine/src/voice_allocator.rs`, `voice.rs` |
| Mod matrix (pitch destination) | `crates/synth_modules/src/mod_matrix.rs`, `crates/synth_core/src/params/mod_matrix.rs` |
| Bus stage (Phase D) | `crates/synth_engine/src/synth_engine.rs:2515-2555`, `docs/channel-strip-c-plan.md` |
| GUI automation picker | `crates/pertylizer/src/gui/sequencer/mod.rs:3375` |
| MCP automation bridge | `crates/pertylizer/src/mcp_bridge.rs:5269-5327` |
| Rack view + module panels (automation badge) | `crates/pertylizer/src/gui/instrument_rack.rs`, `gui/module_panel.rs` |
| Module delete guard | `crates/pertylizer/src/gui/patch_editor.rs:624` (`remove_module`) |
