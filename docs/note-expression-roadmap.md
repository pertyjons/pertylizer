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

- [ ] **Phase A1** — Wire the 6 already-GUI-exposed instrument macros (bugfix; no schema change)
- [ ] **Phase A2** — `AutomationTarget::Module { instrument, module_id, param }` (generic, additive)
- [ ] **Phase B** — Per-note legato/tie + glide fields, driving the existing allocator machinery
- [ ] **Phase C** — Per-note vibrato depth + a small per-note expression block (note expression in miniature)
- [ ] **Phase D** — Shared/bus filter with automatable cutoff (rides on channel-strip Phase 7)
- [ ] **Phase E** — Full Note Expression: per-note custom curves + generic per-note targets + MPE
- [ ] **Parallel track** — `get_project_schema` MCP tool + file-level load-lint (export robustness)

Mark each `- [ ]` as `- [x]` and flip the per-phase **Status** line when it lands.
Keep `docs/history.md` updated against each ship date.

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

- [ ] **Visibility in the Rack view.** A module whose param is automated must show
  it on its panel (`instrument_rack.rs`, `module_panel.rs`) — an *automated* badge
  on the module and ideally on the specific param control, the same way a mod-matrix
  destination reads as "wired". Silent automation that doesn't sound (A1) is one
  trap; automation that sounds but is invisible is the next one.
- [ ] **Deletion / rename guard.** `PatchEditorState::remove_module`
  (`patch_editor.rs:624`) currently tears a module out unconditionally. A module
  referenced by ≥1 automation lane must not vanish silently — either block the
  delete with a warning that lists the referencing lanes, or allow it and convert
  those lanes to a flagged *orphan* state (warn, keep, no-op) so nothing is lost.
  Decide which; the orphan path is also the fallback for projects edited outside
  the GUI.
- [ ] **One reference index.** Both of the above need the same lookup
  (`module_id → [lanes]`). Build it once (sequencer-side, from the automation
  lanes) so the Rack badge and the delete guard can't disagree. Phase D's bus
  filter and Phase E's per-note curves register through the same index.

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
- [ ] **Generalize the base+override split to any `(module_id, param)`.** A2 needs
  the mod-matrix mechanism widened from the fixed destination enum to a generic
  transient override slot the lane writes, applied over the base at process time and
  cleared on stop → param reverts to base. New work; not free in `set_param`.
- [ ] **Define the combine rule per target.** Automation is typically *absolute*
  (the lane value replaces the base while active) whereas mod-matrix is an
  *additive offset*. Decide which each target uses, and what holds before the first
  point / in gaps between lanes (→ base).
- [ ] **Save semantics.** Saving writes the base (knob) value, never the momentary
  automated value — even mid-playback. Ties into the Rack badge: in read mode the
  knob shows the live automated value but still *stores* the base.

---

## Cross-cutting — pitfalls & open design questions
**Applies to:** A1, A2, D, E · these are decisions that must be made *before* the
generic param path ships, not after. Each is verified against current code.

- [ ] **`ModuleId` is positional, not a stable identity.** It is `{ module_type,
  instance: u16 }` (`commands.rs:43`); `apply_mod_offset` even resolves "the Nth
  instance of a type" (`graph.rs:454`). A lane that stores a `ModuleId` silently
  **re-points to a different module** if a same-type module is added/removed/
  reordered — and A1's "first filter in the graph" convention has the identical
  fragility. The reference index (other cross-cutting section) is *not enough*;
  automation needs a stable per-module identity, or a documented, deterministic
  re-resolution rule that survives graph edits.
- [ ] **Discrete / enum params can't be smoothly automated.** `Param` carries
  `choice` params — `FilterMode`, `FilterModel` (`filters.rs:159-173`, decoded via
  `from_index`), oscillator `Waveform`, etc. A continuous normalized lane
  interpolating *through* them is nonsense (halfway between saw and square = ?).
  The descriptor already distinguishes `choice` from continuous — the lane engine
  must read that: continuous → interpolate/smooth, discrete → stepped/quantized.
- [ ] **Not every param is RT-safe to automate.** `grid_size` on the mod matrix
  (`mod_matrix.rs:137`, "Number of modulation slots") *resizes the grid* —
  automating it would heap-allocate on the audio thread. The picker (A2 GUI/MCP)
  must offer only an **allowlist**: continuous *and* RT-safe params. Structural /
  sizing params are not automatable. Decide the flag's home (descriptor).
- [ ] **Control-rate stepping = zipper noise.** The apply path is a hard set —
  `set_volume` (`instrument.rs:749`), `Graph::set_param` (`graph.rs:332`) — with no
  ramp. Per-block automation of cutoff/volume/etc. clicks without smoothing. Decide
  where smoothing lives (per-param ramp in the override layer vs. per-module).
- [ ] **Per-voice fan-out + mid-note seeding.** A lane is *one* per-instrument
  timeline value, but params live per-voice (`handle_set_module_param` writes the
  template *and every live voice*, `synth_engine.rs:1729`). The override must fan
  out to all voices each block, and a voice triggered **mid-sweep** must start at
  the current automated value, not the base — the mod matrix dodges this with a
  per-voice LFO, an instrument lane cannot.
- [ ] **Two controllers, one param.** A filter cutoff can be driven by a mod-matrix
  LFO (additive offset) *and* an automation lane *simultaneously* — the mod matrix
  already targets `FilterCutoff`. Define the order: base → automation (absolute) →
  mod-matrix offset on top? Undefined today; collisions are silent.
- [ ] **Offline render must apply the override identically.** The `analyze_*` tools
  render offline; if the override layer only runs in the live `process()`, analysis
  reads base values, not automated ones — a fresh instance of the known
  "offline reader sees state the live engine never wrote" bug class. Evaluate
  automation in the offline path too, on the same clock.

---

## Phase A1 — Wire the 6 GUI-exposed instrument macros (bugfix)
**Status:** ☐ Not started · **Effort:** S · **Axis:** broken → fix · **Schema:** none

The targets already exist in the enum, the GUI, and MCP; only the playback
dispatch is missing. This is a bugfix of a silent no-op, not a feature.

- [ ] In the `Parameter` dispatch (`synth_engine.rs:2660`), replace the `_ => {}`
  for FilterCutoff/FilterResonance/Attack/Decay/Sustain/Release with resolution to
  the instrument's filter/envelope module. **Apply via the override layer, not the
  destructive `set_param`** — see Cross-cutting "automation value model"; otherwise
  the macro latches and a save corrupts the patch base.
- [ ] Define the resolution convention for instruments with **multiple** filters
  (e.g. "first filter module in the graph"). Document it next to the dispatch.
- [ ] Denormalize `NormalizedValue 0..1` → param range via the existing descriptor
  ranges (descriptor-driven validation already landed, 6ee1c5e).
- [ ] **Migration note for `history.md`:** existing saved projects may already
  contain dead FilterCutoff/ADSR lanes; once wired they begin to *sound* on load —
  a deliberate behavioral change to old projects.

## Phase A2 — `AutomationTarget::Module` (generic param automation)
**Status:** ☐ Not started · **Effort:** M · **Axis:** broken → fix (filter/PWM) · **Schema:** additive

The biggest single faithfulness win: turns the analyzer's already-extracted
per-frame PWM and filter-cutoff contours into exact playback instead of a static
midpoint / fixed-rate LFO guess.

- [ ] Add `AutomationTarget::Module { instrument: SeqInstrumentId, module_id:
  ModuleId, param: Param }` (additive enum variant) in `automation.rs`.
- [ ] Dispatch it through the **override** layer, not `Graph::set_param` — see
  Cross-cutting "automation value model". Reusing the destructive `SetModuleParameter`
  path would latch/corrupt the patch base.
- [ ] GUI: extend the lane target picker (`sequencer/mod.rs:3375`) to browse
  modules + params for the selected instrument.
- [ ] MCP: accept module targets in `build_automation_target`
  (`mcp_bridge.rs:5302`) and surface them in `automation_target_info`.
- [ ] Referential integrity + visibility — see the **Cross-cutting** section
  below. A targeted module must show as *automated* in the Rack view, and it can
  no longer be silently deleted/renamed out from under a live lane.

## Phase B — Per-note legato/tie + glide
**Status:** ☐ Not started · **Effort:** S–M · **Axis:** broken → fix (arpeggio, portamento) · **Schema:** additive

Retires the two worst pitch artifacts (machine-gun arpeggio re-gating; portamento
staircases). The machinery exists in the allocator — this is the sequencer-side
data + wiring.

- [ ] Add `Note.legato`/tie flag and `Note.glide` (target pitch or signed
  semitones + glide time) in `note.rs`.
- [ ] Drive the existing `AllocationMode::Legato` + `GlideState` per note from the
  sequencer playback path.
- [ ] Precedence: per-note glide overrides the instrument allocator default.
- [ ] GUI: piano-roll tie/legato toggle + a glide handle between abutting notes.
- [ ] Export payoff: arpeggio replays as tied notes under one held gate; slides
  replay as glide.

## Phase C — Per-note vibrato + expression block
**Status:** ☐ Not started · **Effort:** S · **Axis:** blander → richer (dead leads) · **Schema:** additive

Vibrato already works at patch level (mod matrix LFO→OscPitch); this makes it
per-note and seeds the expression model.

- [ ] Add a small per-note expression block (vibrato depth/delay, glide time,
  probability…) that scales the existing mod-matrix pitch path.
- [ ] This block **is** note expression in miniature — design its field shape so
  Phase E can extend it rather than replace it.

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
  question" — answering yes absorbs Problems 1b/1d/3 into expression).
- [ ] MPE input mapping (per-note bend/pressure → per-note expression fields).
- [ ] Note Processors layer (arpeggiator / chord / scale-quantize / humanize) —
  optional, for live composition.
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
