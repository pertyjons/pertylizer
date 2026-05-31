# Plan: `suggest_patch_changes` (the last open item)

This was the final ⏸️ **deferred** entry from the (now-completed and removed)
MCP improvements batch — every other item in that batch shipped. This plan
unblocks the one remaining piece.

> **suggest_patch_changes (applicable)** — Concrete per-instrument parameter
> suggestions toward a goal (e.g. "modern_sid", reduce_masking). Fuzzy + large;
> `suggest_music_fixes` partly covers it.

---

## 1. What makes this different from `suggest_music_fixes`

`suggest_music_fixes` (rule engine in `crates/pertylizer/src/analysis/suggest_fixes.rs`)
consumes **analyzer outputs** (harmony / mix-bus / masking / groove / form / hook /
tension) and emits human-readable `FixSuggestion { title, detail, evidence }`. It
**never inspects the instrument voice graph**, and its `CAT_PATCH = "patch"`
category is registered but has **zero rules wired** (verified: no `patch_*` rule
functions, `SuggestionInputs` carries no instrument/module data).

`suggest_patch_changes` is genuinely new and orthogonal:

| Axis              | `suggest_music_fixes`            | `suggest_patch_changes` (new)                    |
|-------------------|----------------------------------|--------------------------------------------------|
| Reads             | analyzer results                 | instrument voice graph (modules + param values)  |
| Output            | prose + evidence                 | **applicable** `{module_id, param, from→to}` ops |
| Actionable        | caller must interpret text       | feed straight into `set_parameter`/`batch_execute` |
| Scope             | whole song / section             | one instrument (or all) toward a goal            |

The "(applicable)" tag in the plan is the key requirement: every suggestion must
be a concrete, executable parameter change, not advice.

---

## 2. Goal vocabulary (designed from scratch — none exists today)

Search confirmed there is no prior "modern_sid" / "reduce_masking" / style /
goal concept anywhere. We define a small, stable, string-keyed set so a caller
can pass filters through directly (same pattern as `ALL_CATEGORIES`):

| goal key         | nature           | inputs needed                          |
|------------------|------------------|----------------------------------------|
| `modern_sid`     | timbral/static   | instrument graph only                  |
| `reduce_masking` | mix-context      | graph + `analyze_masking_matrix`       |
| `tame_harsh`     | timbral/static   | graph (+ optional `analyze_note` peak) |
| `fatten`         | timbral/static   | instrument graph only                  |

Start with **`modern_sid`** and **`reduce_masking`** (the two named in the plan);
the other two are cheap to add once the harness exists and are listed so the
result type / dispatch don't need reshaping later. If scope must shrink, ship
`modern_sid` alone first — it needs no analyzer dependency.

Each goal is a set of small rule functions, mirroring `suggest_fixes.rs` style:
each rule reads the patch, fires only when its precondition holds, and returns
`Option<PatchChange>`. Stable rule IDs (`patch.modern_sid.add_detune`, …) so a
caller can suppress one in a follow-up.

---

## 3. New result type (in `crates/synth_mcp/src/types.rs`)

```rust
/// One concrete, executable parameter change.
pub struct PatchChange {
    pub rule_id: String,          // "patch.modern_sid.add_detune"
    pub instrument_id: u16,
    pub instrument_name: String,
    pub module_id: u32,
    pub module_type: String,      // "osc", "flt", …
    pub param: String,            // parameter key understood by set_parameter
    pub current_value: f32,
    pub suggested_value: f32,
    pub unit: Option<String>,     // from ParameterInfo (Hz, dB, …)
    pub confidence: f32,          // [0,1], descending sort
    pub reason: String,           // one sentence, why
    pub evidence: Vec<String>,    // measured/observed support
}

pub struct SuggestPatchChangesResult {
    pub goal: String,
    pub instrument_ids: Vec<u16>,     // instruments considered
    pub changes: Vec<PatchChange>,    // ranked by confidence
    pub applied: bool,                // true if apply=true ran them
    pub rules_clean: Vec<String>,     // rule IDs whose precondition passed → no change needed
    pub warnings: Vec<String>,
}
```

`PatchChange` is intentionally shaped to drop straight into `set_parameter`
(`instrument_id` + `module_id`/`module_type` + `param` + `suggested_value`), so a
caller can `batch_execute` the whole list — and `rollback` it if unhappy (that
infra already landed).

---

## 4. Bridge layer

**Trait method** — `crates/synth_mcp/src/bridge.rs` (alongside `suggest_music_fixes`):

```rust
fn suggest_patch_changes(
    &self,
    goal: String,
    instrument_id: Option<u16>,        // None = all instruments
    max_suggestions: Option<u32>,
    apply: Option<bool>,               // default false = dry-run / suggest only
    arrangement_start_tick: Option<u64>,   // for reduce_masking window
    arrangement_end_tick: Option<u64>,
) -> Result<SuggestPatchChangesResult, McpBridgeError>;
```

**Implementation** — `crates/pertylizer/src/mcp_bridge.rs`
(`suggest_patch_changes_impl`, dispatched from the trait method like
`auto_gain_stage`):

1. Resolve target instruments (one, or `list_instruments`).
2. For each, snapshot the graph via existing read APIs — **no new measurement
   code**: `get_instrument_info`, `list_modules` (gives `ModuleInfo` with current
   `ParameterInfo` values, ranges, units), and `get_instrument_profiles` (role /
   envelope / register — lets rules be role-aware, e.g. don't brighten a bass).
3. For `reduce_masking`: call `analyze_masking_matrix_impl` over the window, take
   top conflict pairs, map each masked track → its instrument (via
   `set_track_instrument` linkage / track→instrument lookup already used by
   analyzers), and emit cutoff/EQ-band suggestions on the *dominated* instrument.
4. Run the goal's rule set → collect `PatchChange`s, clamp suggested values to the
   parameter's real min/max (from `ParameterInfo`), drop no-op changes
   (`current ≈ suggested`).
5. Sort by confidence, truncate to `max_suggestions` (clamp 1..50).
6. If `apply == Some(true)`: call the existing `set_parameter` path for each
   change, set `applied = true`. Default is dry-run (return ops only).

**Rule module** — new `crates/pertylizer/src/analysis/suggest_patch.rs`, mirroring
`suggest_fixes.rs` structure: a `PatchInputs<'a>` snapshot struct, stable goal
constants, one fn per rule returning `Option<PatchChange>`, and a `suggest_patch()`
dispatcher gated by goal. Keeps the (already 976-line) `suggest_fixes.rs`
untouched and the two engines independent.

### Initial rules

`modern_sid` (graph-only, no analyzer dep):
- `add_detune` — if 2+ oscillators with ~0 detune → small spread for width.
- `pwm_motion` — pulse osc with static pulse width → suggest slow LFO→PWM or a PW offset.
- `filter_resonance` — flat resonance on the main filter → modest bump for character.
- `shorten_release` — very long amp release on a non-pad role → tighten.
- `add_filter_if_missing` — osc→amp with no filter → suggest inserting one (reason-only; flagged low-confidence since it's structural, not a param).

`reduce_masking` (needs `analyze_masking_matrix`):
- `roll_off_masker` — on the dominant instrument of a conflict pair, suggest lowering filter cutoff (mid/high band conflict) toward the masked track's band.
- `hp_the_masked` — high-pass / raise low-cut on the masked instrument when the clash is in sub/low.
- Each carries the conflicting band + `conflict_score` as evidence.

---

## 5. Server layer (`crates/synth_mcp/src/server.rs`)

- **Param struct** `SuggestPatchChangesParam { goal, instrument_id, max_suggestions,
  apply, arrangement_start_tick, arrangement_end_tick }`.
- **Handler**: validate `goal` against the known set (clear error listing valid
  goals on mismatch), then `run_blocking_json()` → bridge.
- **Dispatch macro** entry next to `suggest_music_fixes`:
  `"suggest_patch_changes" => suggest_patch_changes(SuggestPatchChangesParam),`
- **Tool description**: state it returns *applicable* changes the caller can run
  via `batch_execute`, and that `apply=true` performs them in place.

---

## 6. Tests (`crates/pertylizer/tests/`, e.g. new `mcp_suggest_patch_changes.rs`)

Call `*_impl()` directly, the repo convention:
- `modern_sid_on_dual_osc_suggests_detune` — build a 2-osc patch with 0 detune,
  expect a detune `PatchChange` with `from≈0`, `to>0`, within param range.
- `modern_sid_skips_when_already_modern` — patch already detuned/resonant → that
  rule appears in `rules_clean`, not `changes`.
- `suggested_values_respect_param_range` — every change is within the
  parameter's reported min/max (guards the clamp).
- `reduce_masking_targets_dominant_instrument` — two masking tracks → change lands
  on the dominant instrument with the conflict band in `evidence`.
- `apply_true_actually_sets_parameter` — `apply=true` → `get_parameter` reflects
  the new value and `applied == true`.
- `unknown_goal_errors_with_valid_list` — bogus goal → error names valid goals.

---

## 7. Out of scope (state explicitly in result/warnings, defer)

- Structural rewiring beyond a flagged reason-only hint (no auto module insertion
  — `insert_module_between` exists for that as a follow-up).
- ML / preset-corpus matching for "sounds like X". Rules are hand-authored heuristics.
- Goals beyond the four listed; `tame_harsh` / `fatten` can land in a second pass.
- Multi-instrument global balancing (that's `auto_gain_stage` / mix tools' job).

---

## 8. Suggested commit sequence

1. `suggest_patch.rs` rule module + `PatchChange`/`SuggestPatchChangesResult` types
   + `modern_sid` rules + unit tests (graph-only, no analyzer dep). Self-contained.
2. Bridge trait method + `_impl` (resolve instruments, run rules, clamp, dry-run).
3. Server param/handler/dispatch + tool description.
4. `reduce_masking` goal (depends on `analyze_masking_matrix` wiring) + its tests.
5. `apply=true` path + integration test.
6. Update `docs/history.md` (one line), tick the item in
   `plans/mcp-improvements-plan.md`, bump `Cargo.toml` per the `new version` flow.

Each step keeps `cargo fmt --check && cargo build && cargo clippy --all-targets &&
cargo test` green before committing (per CLAUDE.md).
