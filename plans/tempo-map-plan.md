# Tempo-map completion plan (TODO §1.1)

Finish the **tempo map** — the live, per-position tempo mechanism on `Song`.
This is *not* the deleted `AutomationTarget::Global(Tempo)` lane (removed 2026-06-01,
not coming back). The tempo map is a separate, real feature that is already
partly wired; this plan closes the three remaining gaps.

## Current state (grounded — do not rebuild)

Already working:
- **Model** — `TempoChange { tick, bpm }` (`synth_sequencer/src/song.rs:11`), sorted
  private `tempo_changes: Vec<TempoChange>` (`song.rs:112`), `default_tempo` (`:115`).
  API: `set_tempo_at` (`:712`), `tempo_at` (`:723`, **step-only**), `tempo_changes()`
  (`:734`), `remove_tempo_change` (`:745`), `clear_tempo_changes` (`:739`).
- **Engine reads it live** — `sequencer_engine.rs:627/643` do
  `cached_tempo = song.tempo_at(current_tick)` every tick; `process_until_next_tick`
  (`:546`) converts ticks→samples with that BPM. Tempo changes already play back.
- **Persistence** — `tempo_changes` is a plain serde field → already round-trips.
- **GUI editing (rudimentary)** — arrangement ruler right-click `"Set tempo here…"`
  (`arrangement.rs:1275`) with a `DragValue` + `"Remove tempo change here"`; markers
  drawn at `arrangement.rs:1128`. Full undo via `UndoAction::SetTempo` (`undo.rs:166`,
  inverse `:585`), tested (`undo.rs:918/941`).
- **Tests** — `song.rs:1033 test_tick_to_seconds_with_tempo_change`, `:1044 test_tempo_at`
  (asserts the *step* behavior); `project_load_snapshot.rs` records `tempo_change_count`.

The three gaps this plan closes:
1. No MCP tools for the map (only `set_song_tempo` → `default_tempo`).
2. No dedicated GUI tempo lane/curve editor (only a context menu).
3. `tempo_at` is step-only — no accelerando/ritardando ramps.

---

## Phase 1 — MCP tools for the tempo map (closes gap 1)

Cheapest, purely additive, low risk. Wraps existing `Song` API. **Step semantics
only** (no ramp field yet — that arrives in Phase 2 as an additive optional field).

### 1a. Bridge trait + impl
`crates/synth_mcp/src/bridge.rs` — add to the `McpBridge` trait (next to
`set_song_tempo`, `:895`):
```rust
fn set_tempo_at(&self, tick: u64, bpm: f32) -> Result<(), McpBridgeError>;
fn remove_tempo_at(&self, tick: u64) -> Result<bool, McpBridgeError>;
fn get_tempo_map(&self) -> Result<Vec<(u64, f32)>, McpBridgeError>;
```
Impl in `crates/pertylizer/src/mcp_bridge.rs` (mirror `set_song_tempo`, `:1702`):
`song.write().set_tempo_at(Tick(tick), Bpm::new(bpm))` etc. **Do not** send
`EngineCommand::SetTempo` — the engine picks the map up per tick via `tempo_at`
already; only `default_tempo` needs the command. `get_tempo_map` reads
`song.tempo_changes()`.

### 1b. MCP tools (`crates/synth_mcp/src/server.rs`)
Follow the `set_song_tempo` pattern (`:6018`): param struct in `server.rs`
(next to `SetSongTempoParam`, `:2238`), `#[tool(description = …)]` method,
range-validate bpm with `validate_range("tempo", bpm, 20.0, 999.0)`, and register
in the `batch_execute` dispatch table (`:4003`, under the `// Song` block).

- **`set_tempo_at`** — array-first (house convention): param takes
  `Vec<{ tick: u64, bpm: f32 }>` so a whole map can be set in one call. Loop,
  validate each, call bridge. (Matches the array-tool consolidation in the MCP work.)
- **`remove_tempo_at`** — `{ tick: u64 }` (or array); returns how many were removed.
- **`get_tempo_map`** — `NoParams`, returns the sorted list of `{ tick, bpm }`.
  Also fold tempo-map presence into `get_song_info` (`:6006`) so it is discoverable.

Descriptions must state: ticks are absolute (`TICKS_PER_QUARTER` = 960/quarter),
this edits the *map* (not the global default — that stays `set_song_tempo`), and
that today it is a step change (update the wording in Phase 2).

### 1c. Tests
- Round-trip test in `crates/pertylizer/tests/project_load_snapshot.rs`: add a
  fixture whose song has a **non-empty** tempo map (current fixtures are all
  `tempo_change_count: 0`, so persistence is "covered" without ever exercising a
  point). Assert count + values survive save/load.
- Bridge-level test: set → get → remove sequence.

**Deliverable:** AI/automation can add, inspect, and remove tempo-map points.

---

## Phase 2 — Interpolation: accelerando / ritardando ramps (closes gap 3)

The only part with real design risk. `tempo_at` is step-only; make ramps possible
while keeping tick↔time conversion consistent.

### 2a. Data model — ramp mode per point
`song.rs:11` — extend the point:
```rust
pub struct TempoChange {
    pub tick: Tick,
    pub bpm: Bpm,
    #[serde(default)]        // old saves + existing step points load as false
    pub ramp: bool,
}
```
Semantics (**outgoing** ramp, keeps `tempo_at` a local lookup): `ramp = true` on a
point P means *from P.tick, linearly interpolate BPM toward the next point N's bpm
across `[P.tick, N.tick]`, reaching N.bpm exactly at N.tick*. If there is no next
point, hold P.bpm constant. `ramp = false` = step (current behavior). Default
`false` preserves existing songs and keeps `test_tempo_at` green.

Update `set_tempo_at` to take/carry the flag (or add a `set_tempo_ramp_at`);
keep the current 2-arg call working with `ramp: false` default so callers don't
all churn.

### 2b. `tempo_at` — interpolate (`song.rs:723`)
Find preceding point P and following point N. If `P.ramp` and N exists:
```
u    = (tick - P.tick) / (N.tick - P.tick)          // 0..1 in tick space
bpm  = P.bpm + (N.bpm - P.bpm) * u                  // linear in tick
```
else return `P.bpm` (or `default_tempo` before the first point). The engine gets
smooth ramps for free — it already samples `tempo_at` every tick.

### 2c. `tick_to_seconds` / `seconds_to_tick` — ramp-aware integration
**Critical for consistency.** These integrate assuming piecewise-constant tempo
(`song.rs:783/812`); a linear-in-tick BPM ramp is *not* linear in time (the integral
is logarithmic). If `tempo_at` interpolates but these don't, playback/seek and the
time display diverge. Per ramp segment from `(t0,b0)` to `(t1,b1)` with
`K = (t1 - t0) * 60 / TICKS_PER_QUARTER`:

- **Forward** (seconds to traverse the segment), `b1 ≠ b0`:
  ```
  seconds = K * ln(b1 / b0) / (b1 - b0)
  ```
  For a *partial* segment ending at tick `t`, substitute `b(t) = b0 + (b1-b0)·u`,
  `u = (t-t0)/(t1-t0)` as the upper endpoint.
- **Inverse** (`seconds_to_tick`) — solve the same expression for `u`:
  ```
  b(u) = b0 * exp( S * (b1 - b0) / K )      // S = elapsed seconds into segment
  u    = (b(u) - b0) / (b1 - b0)
  tick = t0 + u * (t1 - t0)
  ```

**Numerical stability (do not use the naïve forms above verbatim).** As
`b1 → b0` (a nearly-flat ramp, or a step segment where `b1 == b0`), the
`b1 - b0` denominator and the `ln(b1/b0)` numerator both go to zero →
catastrophic cancellation, `NaN`, or divide-by-zero. Guard with a small-diff
fallback plus `ln_1p`/`exp_m1`:

```rust
// forward
let diff = b1 - b0;
if diff.abs() < 1e-5 {
    seconds += k / ((b0 + b1) * 0.5);            // flat/step → constant-tempo average
} else {
    seconds += k * (diff / b0).ln_1p() / diff;   // ln(b1/b0) = ln_1p((b1-b0)/b0), stable
}

// inverse
let diff = b1 - b0;
let u = if diff.abs() < 1e-5 {
    b0 * s / k
} else {
    b0 * (s * diff / k).exp_m1() / diff          // exp_m1 = e^x - 1, stable
};
```

This same `< 1e-5` branch is what the plan means by "step segments degenerate to
the constant formula" — there is no separate `b1 == b0` path. BPM is validated
`≥ 20`, so `b0`/`b1` are always positive; still `debug_assert!` it.

**Loop carry (`song.rs:783/812`).** The existing loops thread `current_tick` /
`current_tempo` forward; add `current_ramp: bool` the same way — it is the ramp
flag of the point that *starts* the current segment (init `false`: the pre-first-point
segment on `default_tempo` is always constant). Then:
- **Break case** (`change.tick >= target`): `target` lies inside the segment ending
  at `change`. If `current_ramp`, integrate the partial ramp with `b1 = change.bpm`;
  else constant.
- **After the loop** (`target` past all changes): there is no next point, so tempo is
  constant at `current_tempo` regardless of `current_ramp`.

### 2c-note. Engine discretization drift is negligible — no sample-level integration
The engine samples `tempo_at` once per tick and holds it constant across the tick
(`sequencer_engine.rs:546`), i.e. a Riemann sum, while `tick_to_seconds` is the exact
integral. The gap depends only on the segment's endpoint BPMs, not its length:
`≈ (60 / 2·TPQ)·(1/b1 − 1/b0)` — for an extreme 60→180 BPM ramp that is ≈ −0.35 ms.
Well below audible/scheduling relevance, so **do not** add continuous sample-rate
tempo integration to the audio thread; the per-tick sampling stays.

### 2d. MCP + tests
- Extend the Phase-1 `set_tempo_at` param with an optional `ramp: bool`
  (`#[serde(default)]`) — additive, no breaking change. Update tool descriptions
  ("step or linear ramp to the next point").
- Tests in `song.rs`: a ramp segment's `tempo_at` at the midpoint equals the mean
  BPM; `tick_to_seconds` over a known ramp matches the closed form; round-trip
  `seconds_to_tick(tick_to_seconds(t)) == t` across a ramp; the existing step tests
  stay green.

**Deliverable:** real tempo automation (accelerando/ritardando) that plays back and
seeks consistently.

---

## Phase 3 — GUI tempo lane / curve editor (closes gap 2)

Replace the context-menu-only editing with a proper draggable lane. Builds on the
existing marker drawing (`arrangement.rs:1128`) and data collection (`:113`).

> **Status (partial).** The low-risk, testable parts landed without an in-app
> eyeball: the undo model now carries ramp (`UndoAction::SetTempo { old, new:
> Option<(Bpm, bool)> }`), the ruler-context-menu gained a **"Ramp to next"**
> toggle (applies immediately — the song is the source of truth, so no
> immediate-mode per-frame-local persistence bug), and the ruler draws a ramp
> cue (a `→` label + a slope-signed connector to the next point). The
> **draggable curve lane itself (3a/3b geometry) is deferred** to a dedicated
> in-app session where the band-insertion + drag feel can be iterated live —
> creating a ramp today is the 2-step "Set tempo here → tick the toggle".

### 3a. Data
Widen the collected tuple `Vec<(u64, f32)>` → `Vec<(u64, f32, bool)>` (tick, bpm,
ramp) in `ArrangementData` (`arrangement.rs:113`).

### 3b. Lane rendering + interaction (`arrangement.rs`)
A dedicated horizontal tempo strip (or a taller ruler band):
- Map bpm to vertical position within a visible range (e.g. 20–300 BPM → lane
  height); tick → horizontal (reuse the arrangement's tick↔x mapping).
- Draw each point as a draggable handle; connect points with a polyline that
  **shows the curve form**: step points draw flat-then-jump, `ramp` points draw a
  sloped line to the next point — so the user sees what they hear.
- **Drag** a handle → snap tick, clamp bpm, apply via `set_tempo_at` (remove old
  tick + insert new when the tick moved). **Double-click** empty lane → add a point.
  Right-click point → remove / toggle step↔ramp (reuse the existing menu path at
  `:1275`).
- **Clamp horizontal drag between neighbors** so a point can never cross an adjacent
  one and reorder the list: `tick ∈ [prev.tick + 1, next.tick - 1]`. Keeps the sorted
  invariant and the interaction stable/intuitive.

### 3c. Undo
`UndoAction::SetTempo` (`undo.rs:166`) currently carries `{ tick, old_bpm, new_bpm }`.
Extend the payload to also carry the ramp flag (e.g. `old/new: Option<(Bpm, bool)>`) —
this one shape covers create / change-BPM / change-ramp / remove uniformly. Add a move
variant (or model a drag as remove-old + set-new) so tick changes undo correctly.
Update the inverse logic (`undo.rs:585`) and its tests (`undo.rs:918/941`).
**Push exactly one undo entry per completed drag** — on `drag_released()`, not per
frame — or the history floods with per-pixel micro-steps. (The existing context-menu
path already pushes one entry per Apply; only the new drag interaction needs this.)

**Deliverable:** hand-editable tempo curve in the arrangement, at parity with the
automation lanes.

---

## Sequencing & rationale

1. **Phase 1 first** — additive, low-risk, immediately unlocks AI-driven tempo-map
   editing; nothing else depends on it.
2. **Phase 2** — the feature with the most musical payoff and the only real design
   risk (the `tick_to_seconds`/`seconds_to_tick` consistency). The `ramp` field it
   introduces is inert until this phase, so it lands with its meaning.
3. **Phase 3** — most UI work; benefits from the Step/Ramp model already existing so
   the editor can draw the correct curve shape.

Each phase is independently shippable and testable. Follow the CLAUDE.md build gate
(`cargo fmt --check`, `build`, `clippy --all-targets`, `test`) and MCP-feedback rule
per phase. No backward-compatibility constraints apply (active development), but the
`#[serde(default)]` on `ramp` is a free win that keeps existing saves loading.

## Deferred / out of scope

- **`O(log M)` cumulative-seconds cache (deferred, trigger-based).** `tick_to_seconds`
  / `seconds_to_tick` loop over all `M` tempo changes per call. A `#[serde(skip)]
  cumulative_seconds: f64` on `TempoChange`, recomputed once per mutation, would make
  both a `partition_point` + `O(1)` closed form. Real, but **CLAUDE.md §5 forbids
  optimizing before it is a *measured* problem** — this plan does not make the existing
  `O(M)` worse, and hundreds-of-points maps (dense live-recording tempo maps) are
  hypothetical here. Do it only if profiling shows these calls as a hot path; until
  then keep the simple loop.
- **MIDI export ramp discretization (future, only if MIDI export is added).** Standard
  MIDI (`FF 51 03`) has no continuous tempo ramps. If Pertylizer ever exports `.mid`,
  ramps must be discretized into a series of dense tempo steps (e.g. one per beat /
  every ~240 ticks) or other DAWs will replay an accelerando as a single tempo jump.
  No MIDI export exists today, so this is a note, not a task.
