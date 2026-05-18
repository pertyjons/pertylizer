# Move project I/O off the GUI thread

**Status:** Complete, 2026-05-18 (five sessions, ~1500 LOC).
**Driver:** TODO §0.1 — MCP `new_project` / `save_project` / `load_project` time out
when the Pertylizer window is minimized, hidden, or unfocused. Today the I/O is
serialized through `pending_project_action`, which is only drained inside
`eframe::App::ui()` (`gui/egui_backend.rs:689`). When eframe pauses repaints,
the queue is never drained and `submit_project_action`'s 30 s condvar wait
expires, killing the MCP session.

This document is the working plan for **Fix B** from the design discussion:
run project I/O on the MCP worker thread, let the GUI mirror the result
reactively on next frame.

### Outcome

The bug is fixed. MCP `save_project` / `load_project` / `new_project` execute
on the MCP worker thread (with `block_in_place`) regardless of GUI focus state.
The GUI consumes a `pending_project_refresh` marker on its next frame and
rebuilds its UI mirrors via `refresh_ui_from_project`. `submit_project_action`,
`pending_project_action`, `project_action_result`, the `ProjectAction` enum,
and the headless `spawn_worker` are all removed. Both GUI and MCP routes flow
through one canonical engine-apply (`project_apply::apply_project`), one
canonical builder (`project_apply::build_project_from_engine` + GUI overlay
for positions/groups/canvas/color/visualizers), and one canonical UI-refresh
(`SynthApp::refresh_ui_from_project`). Round-trip + snapshot + engine-state +
headless-bridge test suites stay green throughout.

## Progress log

- **Session 1 (Phase 0) — complete.** Round-trip + snapshot + headless-bridge
  baseline tests landed. 13 example projects covered. Three `#[ignore]`d MCP
  load/save/new tests document the bug; un-ignored after Phase 3.
- **Session 1 discovery — `crates/pertylizer/src/project_apply.rs` already
  exists** (868 lines) with `apply_project`, `reset_to_new_project`,
  `load_file_into_engine`, `save_project_to`, plus a `pub fn spawn_worker`
  that drains `pending_project_action` on its own thread. It was wired into
  `main.rs::run_headless_mcp` but not into `run_gui`. The refactor plan
  below was rewritten to leverage this existing module rather than create
  it from scratch.
- **Session 2 (Phase 1) — complete.** Reconciled `apply_project` to send
  `SetInstrumentEnabled(true)` per instrument and `SetFocusedInstrument` at
  the end. Promoted `apply_project` / `reset_to_new_project` /
  `load_file_into_engine` / `save_project_to` from `pub(crate)` to `pub`.
  Extracted `SynthApp::refresh_ui_from_project` and `refresh_ui_after_reset`
  from the old `load_project_data` body. `load_project_data` /
  `reset_to_new_project` are now thin wrappers calling `apply_project` +
  `refresh_*`. Added `patch_bridge::populate_editor_from_patch` (UI-only).
  Pure refactor — no observable behavior change.
- **Session 3 (Phase 2 + 3) — complete. Bug fixed.** Added
  `pending_project_refresh`, `project_revision`, `last_loaded_project_path`,
  `last_project_io_status`, `project_io_lock`, and `author` to
  `McpSharedState`; promoted `awe_description` to `Arc<Mutex<String>>`.
  Rewrote `AppSynthBridge::{new_project, save_project, load_project}` to
  call `project_apply::*` directly under `block_in_place`. Replaced the
  GUI's `pending_project_action` polling block with a
  `pending_project_refresh` consumer. Deleted `submit_project_action`,
  `pending_project_action`, `project_action_result`, the `ProjectAction`
  enum, and the headless `spawn_worker`. Un-ignored the three MCP
  load/save/new tests and strengthened them with concrete assertions.
- **Session 4 (Phase 4 + 5) — complete.** Added `ProjectBuildOptions` so
  one builder (`build_project_from_engine`) serves both GUI and MCP.
  Refactored `create_project_from_app` from ~105 lines to ~17, layered
  on `overlay_ui_metadata` (color / positions / groups / canvas / visualizers).
  GUI now surfaces MCP-initiated I/O outcomes in the status line via a
  per-frame poll of `last_project_io_status`. Author sync from project
  file to `shared.author` lands in both `apply_project`'s GUI refresh and
  the MCP bridge so cross-path saves see fresh metadata.
- **Session 5 (Phase 6) — complete.** TODO §0.1 entry marked done with
  follow-up filed for the remaining `pending_*` queues. `history.md`
  entry under `[unreleased]`. Plan doc closed out.

### Behavior differences between `apply_project` and `load_project_data`

Audited during Session 1. These must be reconciled before `load_project_data`
can safely delegate to `apply_project`:

| # | `load_project_data` (GUI) | `apply_project` (headless) | Resolution |
|---|---|---|---|
| 1 | `session.reset_counters_for_instrument` per instrument | not called | Add to `install_instrument` in `project_apply` |
| 2 | Visualizer modules registered with `Arc<VisualizationBuffer>` via `EngineHandle.visualization_buffers` | silently skipped (`VisualizerRequiresGui`) | Visualizer registration stays on the GUI side; `apply_project` already documents the skip. `refresh_ui_from_project` adds visualizers after apply. |
| 3 | `EngineCommand::SetInstrumentEnabled(true)` per instrument | not sent | Add to `install_instrument` |
| 4 | `keyboard.set_octave_offset(patch.settings.octave_offset)` per instrument (last instrument wins) | not touched (keyboard is GUI) | Move to `refresh_ui_from_project` |
| 5 | No wait after instrument adds | `wait_for_instrument_count(2000ms)` for snapshot to settle | Acceptable in GUI; harmless latency |
| 6 | `awe_description` written into `mcp_shared.awe_description` | not touched | Add to `apply_project` (Category-A shared state) |
| 7 | `author` field consumed and stored on `SynthApp` | not touched | Phase 2.4 promotes `author` to `Arc<Mutex<Option<Author>>>`; both paths read/write the same Arc |
| 8 | `handle.set_focused_instrument(active_id)` per load | not sent | Add to `apply_project` (route via `EngineCommand::SetFocusedInstrument`) |

---

## 1. Goal

Eliminate the GUI handoff for project save / load / new. After this work:

- `mcp_bridge::save_project / load_project / new_project` execute on the MCP
  worker thread (or a dedicated blocking pool, via `block_in_place`).
- The GUI's "File → Open" / "File → Save" use the **same** pure I/O entry
  points — single code path for both MCP and GUI to prevent drift.
- `pending_project_action`, `project_action_result`, and `submit_project_action`
  are removed.
- The MCP session survives a save/load even when the window is minimized,
  hidden, App-Napped (macOS), or briefly hung.

---

## 2. Out of scope

- Other `pending_*` queues (`pending_patch`, `pending_awe_state`,
  `pending_auto_layout`). They have the same architectural shape but are
  cheaper to fix individually once the pattern is established here.
- The remaining `block_in_place` gap for ~13 other MCP tools (TODO §0.1
  follow-up). Tracked separately.
- Unifying scattered shared state into a single `Arc<RwLock<ProjectRuntimeState>>`.
  Mentioned as future direction in §9; not required for this plan.

---

## 3. State ownership target

Today every load mutates ~30 fields across `SynthApp`. The refactor splits them
in two:

### Category A — "true project data" (moves to / stays in shared state)

| Field | Lives where today | After refactor |
|---|---|---|
| Song (tracks/patterns/arrangement) | `Arc<RwLock<Song>>` (already shared, `main.rs:100`) | Same |
| Instruments / patches / modules | Owned by `session: Arc<SynthSession>` | Same |
| Sample library | `Arc<RwLock<SampleLibrary>>` (already shared, `main.rs:112`) | Same |
| AWE state | `mcp_shared.awe_state: Mutex<AweState>` | Same |
| AWE description | `mcp_shared.awe_description: Mutex<String>` | Same |
| Master volume / glide / tempo | Sent via `EngineCommand::*` | Same |
| Per-instrument params (description, pan, mute, solo, midi_channel, sidechain, allocation mode, oversampling, key range, transpose, velocity sensitivities) | Set via `session.set_*` methods (all MCP-callable today) | Same |
| Active instrument id | `EngineCommand::SetActiveInstrument` | Same |
| Octave offset (engine view) | `GlobalProjectState.octave_offset` | Same — GUI keyboard widget reads from shared state on refresh |
| `author: Option<Author>` | `SynthApp.author` (UI-only today, written back at save) | **New:** `McpSharedState.author: Mutex<Option<Author>>`. Promoting it makes it MCP-readable/writable and removes the last GUI-only "project metadata" field. See §2.4. |

### Category B — "UI-only mirror state" (stays on GUI thread, refreshed reactively)

| Field | Today | After refactor |
|---|---|---|
| `current_project_path` | `SynthApp` | GUI reads `shared.last_loaded_project_path` on refresh |
| `current_patch_name` | `SynthApp` | GUI re-derives from active instrument's patch on refresh |
| `current_patch_path` | `SynthApp` | Cleared on project load (MCP load never sets it) |
| `dirty` flag | `SynthApp` | Cleared on refresh |
| `sample_view_state.peaks` cache | `SynthApp` | Invalidated on refresh |
| Undo manager | `SynthApp` | Cleared on refresh |
| Patch-editor canvas state (per-instrument zoom/pan) | `InstrumentUiState.patch_editor` | Re-derived from `patch.settings.canvas_size` on refresh (already happens on patch load) |
| Dialog state, panel state | `SynthApp` | Untouched (transient anyway) |

### Category C — Engine commands (already MCP-callable, no work)

`SetMasterVolume`, `SetGlideTime`, `SetTempo`, `SetSong`, `SetAweEnabled`,
`SetAweParameter`, `SetAweState`, `SetInstrumentParameter` (8 variants),
`RenameInstrument`, `LoadSampleData`, `Stop`, `SetActiveInstrument`.

All routed via `session.command_sender()` which the MCP bridge already holds.

---

## 4. Risks and mitigations

| Risk | Mitigation |
|---|---|
| Audio thread reads `song` via `try_read()`; MCP write-lock during load → one buffer of silence | Build new `Song` outside the lock, swap with short write. Send `EngineCommand::Stop` before mutation as today. |
| Many short-lived `session.add_instrument` / `set_*` calls during load → audio thread sees a half-loaded project for ~ms | Acceptable today (same as GUI path). If problematic, add `session.begin_batch / commit_batch` later — out of scope. |
| GUI shows stale state if it doesn't refresh promptly | Per-load version counter on `McpSharedState`; GUI polls it cheaply (atomic load) every frame and refreshes when incremented. |
| Sidechain `prev_outputs` cache holds stale `InstrumentId` after instrument set is replaced | Already handled by `SynthEngine::remove_instrument` clearing entries. Verify in test (§5 Phase 0). |
| `keyboard.set_octave_offset` was called by GUI on load; if GUI refresh races with first key press, octave may be wrong for one keystroke | Keyboard widget reads octave from shared `GlobalProjectState` on every frame instead of caching. One-line change. |
| File-IO error path: today the GUI shows a dialog. After refactor, only MCP sees the error; GUI never knows | Store last load error in `McpSharedState.last_project_io_status: Mutex<Option<Result<String, String>>>`; GUI reads + shows in status line when it changes. |
| Two clients (MCP + GUI) issue concurrent `load_project` | Serialize via `Mutex<()>` on `McpSharedState.project_io_lock`. Second waiter blocks briefly, runs after first completes. |

---

## 5. Plan — phases and priorities

Tests first (per request). Each phase must leave the tree green (`cargo build`,
`cargo clippy --all-targets`, `cargo test`, `cargo fmt --check`).

### Phase 0 — Lock in current behavior (TESTS, no production change)

Goal: be able to detect regressions before refactoring.

- **0.1 Data-layer round-trip tests**
  New test file `crates/pertylizer/tests/project_io_round_trip.rs`. For each
  bundled example project under `assets/examples/projects/*.json` and `*.zip`:
  1. Load file via `project::load_file`.
  2. Save to a `tempfile::NamedTempFile` via `ProjectFile::save` (or
     `bundle::save_bundle`).
  3. Load again.
  4. Assert deep equality of `ProjectFile` (derive `PartialEq` if missing on
     any contained type).
  Covers ~6 examples (vary in: with/without samples, with/without AWE,
  sidechain present, automation present).

- **0.2 Snapshot fixture per example project**
  New test file `crates/pertylizer/tests/project_load_snapshot.rs`. For each
  example: load → serialize `(Song, Vec<InstrumentState>, GlobalProjectState,
  awe_description, sample_library_summary)` to a stable JSON shape →
  compare against committed fixture under
  `crates/pertylizer/tests/fixtures/project_snapshots/<name>.json`.
  First run writes the fixture (gated by `INSEKT_UPDATE_SNAPSHOTS=1` env var,
  same pattern as snapshot tests in the workspace if any; otherwise plain
  fail-with-diff on mismatch).
  Purpose: the same comparison runs **after** refactor to prove the new
  apply path produces identical state.

- **0.3 Headless load-via-MCP-bridge test**
  New test file `crates/pertylizer/tests/mcp_project_load.rs`. Constructs an
  `AppSynthBridge` with the same shared types as `main.rs` but **without** an
  egui app. Today this will fail or hang on the condvar — that's expected
  and documents the bug. Mark as `#[ignore]` with a comment explaining it
  will be un-ignored after Phase 3. The test that DOES pass: a "bridge
  constructs cleanly, song is initially empty, set_tempo via MCP mutates
  shared Song" baseline.

- **0.4 Add `PartialEq` derives where missing**
  Audit `Song`, `InstrumentState`, `Patch`, `GlobalProjectState`, `AweState`,
  `SampleLibrary` summary. Add `#[derive(PartialEq)]` (and `Eq` where it
  applies) on whatever's needed for §0.1 to compile.

Exit criterion: Phase 0 tests green, fixtures committed, snapshot test
documented in `docs/history.md` under "unreleased" as a no-op baseline.

### Phase 1 — Reconcile + extract UI-refresh (no observable behavior change)

Goal: leverage the existing `project_apply` module so both the GUI and (future)
MCP-direct callers share the same engine-apply path. Extract the UI-mirror
portion of `load_project_data` into a separate canonical function. Pure
internal refactor — no user-visible change.

- **1.1 Engine-state regression tests.** New
  `crates/pertylizer/tests/project_apply_engine_state.rs`. For two
  representative example projects (one minimal, one with sidechain), drive
  the apply via *both* paths and snapshot the post-apply engine state:
  instrument-by-instrument `InstrumentSnapshot` fields (volume, pan, mute,
  solo, category, key_range, transpose, oversampling, allocation_mode,
  stealing_strategy, max_voices, velocity sensitivities, sidechain_source_id,
  midi_channel, descriptions), module counts/connections per instrument,
  effect-chain order, `Song` tempo/pattern/track counts, master volume.
  These tests pass under the GUI path today and must pass under the
  delegated path after §1.4. Any divergence is a reconciliation bug.

- **1.2 Reconcile `project_apply::apply_project`** to match the GUI's
  observable engine writes:
    - Call `session.reset_counters_for_instrument(inst_id)` after
      `session.add_instrument_with_id_and_config`.
    - Send `EngineCommand::SetInstrumentEnabled { enabled: true }` per
      instrument (mirrors what `patch_bridge::load_patch` did).
    - Send `EngineCommand::SetFocusedInstrument(active_id)` at the end.
    - Write `project.global.awe_description` into the
      `Arc<Mutex<String>>` `awe_description` slot.
    - Visualizers stay skipped (documented as `VisualizerRequiresGui`);
      `refresh_ui_from_project` adds them on the GUI side after apply.

- **1.3 Promote internal API surface.** Change `pub(crate)` to `pub` on
  `apply_project`, `reset_to_new_project`, `load_file_into_engine`, and
  `save_project_to`. Add a `pub fn apply_project_from_disk(path, &Self)`
  convenience that wraps load + apply.

- **1.4 Add `SynthApp::refresh_ui_from_project(&ProjectFile)`** —
  extracts the UI-only portion of `load_project_data` into one canonical
  function:
    - Remove visualizers for old instruments; clear
      `handle.visualization_buffers`.
    - Clear `self.instruments`; rebuild `InstrumentUiState` per instrument
      with all UI-mirror fields (volume, pan, mute, solo, key_range,
      transpose, oversampling, category, description, color,
      allocation_mode, max_voices, velocity sensitivities,
      sidechain_source_id, patch_description, canvas_size,
      patch_editor population).
    - `patch_bridge::populate_editor_from_patch(...)` — new helper, pure
      UI-side patch loading (adds modules + connections to `PatchEditor`,
      registers visualizer buffers, no other engine commands; engine has
      already been written by `apply_project`).
    - Set `self.glide_time`, `self.awe_ui`, `self.awe_enabled`,
      `self.current_project_author`, `self.next_instrument_id`,
      `self.active_instrument_id`, `keyboard.set_octave_offset` from
      project.

- **1.5 Add `SynthApp::refresh_ui_after_reset()`** — the corresponding
  UI-only reset for "new project".

- **1.6 Refactor `load_project_data` and `reset_to_new_project`** to thin
  wrappers:
    ```rust
    fn load_project_data(&mut self, project: ProjectFile) {
        let ctx = /* build ProjectApplyContext from self.* */;
        let _ = project_apply::apply_project(&project, &self.session,
                                              &self.song, &self.sample_library);
        self.refresh_ui_from_project(&project);
    }

    fn reset_to_new_project(&mut self) {
        let _ = project_apply::reset_to_new_project(&self.session, &self.song,
                                                     &self.sample_library);
        self.refresh_ui_after_reset();
    }
    ```

Exit criterion: Phase 0 tests still green. New §1.1 engine-state tests pass
under both old and new code paths. No user-visible change in GUI mode.

### Phase 2 — Concurrency primitives + refresh queue

- **2.1 Add to `McpSharedState`** (`mcp_shared.rs`):
  ```rust
  pub project_io_lock: parking_lot::Mutex<()>,           // serializes save vs load
  pub project_revision: AtomicU64,                       // bumped on every apply
  pub pending_project_refresh: Mutex<Option<ProjectRefresh>>,
  pub last_loaded_project_path: Mutex<Option<PathBuf>>,
  pub last_project_io_status: Mutex<Option<Result<String, String>>>,
  ```
  where:
  ```rust
  pub enum ProjectRefresh {
      Loaded(Box<ProjectFile>),
      Reset,
  }
  ```
- **2.2 Promote `awe_description` to `Arc<Mutex<String>>` field**
  (currently `Mutex<String>`) so it can be passed into `apply_project`
  without holding `Arc<McpSharedState>`. Same exercise as Phase 2.4's
  `author`.
- **2.3 Promote `author` to shared state.** Add
  `pub author: Arc<Mutex<Option<Author>>>` to `McpSharedState`. Today
  `SynthApp.current_project_author` is the last project-level metadata
  that's GUI-only — moving it now means `apply_project` writes it from
  the loaded `ProjectFile` and `save_project_to` reads it back. The GUI's
  "Project → Edit metadata…" dialog reads/writes the same Arc. Unblocks
  future `get_project_author` / `set_project_author` MCP tools (trivial
  follow-up once the field is shared).

No test additions in this phase — coverage comes from §3.5.

### Phase 3 — MCP bridge direct, remove `submit_project_action`

After Phase 1 there is a canonical `apply_project`; Phase 3 wires the MCP
bridge to call it directly instead of routing through the GUI.

- **3.1 Rewrite `AppSynthBridge::save_project`** (`mcp_bridge.rs:2402`)
  to acquire `project_io_lock`, build the `ProjectFile` from shared state
  (use `project_apply::save_project_to`'s internal builder, exposed as a
  helper), write the file directly (bundle if samples present), set
  `last_project_io_status = Ok`. Wrap in `block_in_place` (already done
  at `server.rs:4515`). **Remove the `submit_project_action` call.**

- **3.2 Rewrite `AppSynthBridge::load_project`** (`mcp_bridge.rs:2407`)
  to acquire `project_io_lock`, call `project::load_file`, then
  `project_apply::apply_project` directly, then stash
  `ProjectRefresh::Loaded(project.clone())` into
  `pending_project_refresh`, bump `project_revision`, set
  `last_loaded_project_path`, set `last_project_io_status = Ok`. Wrap
  in `block_in_place` (already done at `server.rs:4528`).

- **3.3 Rewrite `AppSynthBridge::new_project`** (`mcp_bridge.rs:2398`)
  to call `project_apply::reset_to_new_project` directly, stash
  `ProjectRefresh::Reset`, bump revision, clear
  `last_loaded_project_path`. Wrap the MCP tool in `block_in_place`
  (currently NOT wrapped at `server.rs:4501` — fix on this commit).

- **3.4 Replace GUI polling.** In `egui_backend.rs:689-752`, replace the
  `pending_project_action` consumer with a `pending_project_refresh`
  consumer:
  ```rust
  if let Some(refresh) = shared.pending_project_refresh.lock().ok()
      .and_then(|mut p| p.take()) {
      match refresh {
          ProjectRefresh::Loaded(project) => {
              self.current_project_path =
                  shared.last_loaded_project_path.lock().ok()
                       .and_then(|g| g.clone());
              self.refresh_ui_from_project(&project);
          }
          ProjectRefresh::Reset => self.refresh_ui_after_reset(),
      }
  }
  ```

- **3.5 Delete** `submit_project_action`, `pending_project_action`,
  `project_action_result`, `ProjectAction` enum from `McpSharedState`
  and `mcp_shared.rs`. Update the headless `spawn_worker` to drain the
  new path (or remove the worker entirely — MCP bridge now does the
  work synchronously).

- **3.6 Un-ignore** the three MCP tests in
  `tests/mcp_project_load.rs` (`new_project_works_without_gui`,
  `load_project_works_without_gui`, `save_project_works_without_gui`).
  Strengthen with concrete assertions: `project_revision` bumped,
  `last_loaded_project_path` set, `Song.default_tempo` matches loaded
  value, `session.list_instruments()` matches fixture count.

Exit criterion: full test suite green including the un-ignored MCP
tests. Manual smoke: minimize Pertylizer, drive `load_project` over MCP,
verify it succeeds in < 1 s and GUI shows the loaded project when
brought back to front.

### Phase 4 — Status-line polish

Phase 3 already wires `pending_project_refresh` for the UI to consume.
Phase 4 is now small.

- **4.1 Show I/O errors in status line** when `last_project_io_status`
  transitions to `Err(...)` (compare against last-seen value).

- **4.2 Dirty-flag handling.** A successful MCP load clears `self.dirty`
  inside `refresh_ui_from_project` (consistent with how GUI loads
  behave). A successful MCP save also clears it. Both paths already
  share `refresh_ui_from_project`, so this lands automatically — just
  add the explicit assignment.

Exit criterion: error path verified manually.

### Phase 5 — GUI's own File-menu uses the new path

- **5.1 "File → Open"** calls `project_apply::load_file_into_engine`
  (the same function MCP uses) and then `refresh_ui_from_project`.

- **5.2 "File → Save / Save As"** calls `project_apply::save_project_to`
  (or bundle equivalent). Removes `SynthApp::create_project_from_app`'s
  remaining role.

- **5.3 "File → New"** calls `project_apply::reset_to_new_project` then
  `refresh_ui_after_reset`.

- **5.4 Delete dead code** in `egui_backend.rs`:
  `create_project_from_app` (or shrink to a one-liner that delegates),
  the per-instrument apply loop inside the old `load_project_data`,
  visualizer cleanup that's now in `refresh_ui_from_project`.

Exit criterion: single I/O path used by both MCP and GUI. Phase 0 tests
still green.

### Phase 6 — Cleanup, docs, history

- Update `docs/TODO.md`:
  - Remove §0.1's "MCP timeout when window minimized" entry (replaced).
  - Note that the same fix template applies to `pending_patch`,
    `pending_awe_state`, `pending_auto_layout` (filed as follow-up).
- Update `docs/history.md` under `[unreleased]`: one or two sentences,
  per `CLAUDE.md` style.

---

## 6. Acceptance criteria

1. `cargo build`, `cargo clippy --all-targets`, `cargo test`,
   `cargo fmt --check` — zero warnings.
2. `tests/project_io_round_trip.rs` passes — every bundled example loads,
   saves, reloads, deep-equals.
3. `tests/project_load_snapshot.rs` passes against committed fixtures
   before AND after the refactor (proves no behavior change).
4. `tests/mcp_project_load.rs` passes (un-ignored) without any egui app
   running.
5. Manual smoke: with Pertylizer minimized, MCP `load_project` returns in
   < 1 s; GUI updates next time it's brought to front (or in real-time
   on Linux/Windows).
6. `pending_project_action`, `project_action_result`,
   `submit_project_action` removed from the tree.

---

## 7. File-by-file checklist

| File | Phase | Change |
|---|---|---|
| `crates/pertylizer/tests/project_io_round_trip.rs` | 0.1 | **done** (Session 1) |
| `crates/pertylizer/tests/project_load_snapshot.rs` | 0.2 | **done** (Session 1) |
| `crates/pertylizer/tests/fixtures/project_snapshots/*.json` | 0.2 | **done** (Session 1) |
| `crates/pertylizer/tests/mcp_project_load.rs` | 0.3 / 3.6 | **done** (Session 1, 3 tests still `#[ignore]`d for Phase 3) |
| `crates/pertylizer/tests/project_apply_engine_state.rs` | 1.1 | new — engine-state regression tests |
| `crates/pertylizer/src/project_apply.rs` | 1.2 / 1.3 | reconcile semantics, promote visibility |
| `crates/pertylizer/src/gui/patch_bridge.rs` | 1.4 | new `populate_editor_from_patch` (UI-only) |
| `crates/pertylizer/src/gui/egui_backend.rs` | 1.4–1.6 / 3.4 / 4 / 5 | add `refresh_ui_from_project`, `refresh_ui_after_reset`; thin out `load_project_data` / `reset_to_new_project`; replace polling block; File-menu wiring |
| `crates/pertylizer/src/mcp_shared.rs` | 2 / 3.5 | add new fields (incl. `author`, `pending_project_refresh`, `project_revision`), promote `awe_description` to `Arc<Mutex<…>>`, remove old `pending_project_action` / `project_action_result` |
| `crates/pertylizer/src/mcp_bridge.rs` | 3.1–3.3 | rewrite three handlers, remove `submit_project_action` |
| `crates/synth_mcp/src/server.rs` | 3.3 | wrap `new_project` in `block_in_place` |
| `crates/pertylizer/src/main.rs` | 3.5 | remove headless `spawn_worker` call if worker deleted |
| `docs/TODO.md` | 6 | mark §0.1 done, file follow-ups |
| `docs/history.md` | 6 | one-line entry under `[unreleased]` |

---

## 8. Order of execution (tight-feedback-loop pacing)

Five sessions, each leaves tree green:

1. **Session 1 — Phase 0 (DONE).** Round-trip + snapshot + headless-bridge
   baseline tests landed. No production behavior touched.
2. **Session 2 — Phase 1.** Engine-state regression tests, reconcile
   `apply_project` semantics, extract `refresh_ui_from_project` +
   `refresh_ui_after_reset`, thin out `load_project_data` and
   `reset_to_new_project` to wrappers. Pure refactor, no observable
   behavior change.
3. **Session 3 — Phase 2 + 3.** Add `pending_project_refresh`,
   `project_revision`, `author` shared. Rewrite MCP bridge to call
   `apply_project` directly with `block_in_place`. GUI polls
   `pending_project_refresh`. Delete `submit_project_action` and
   friends. Un-ignore the three MCP tests. **This is the session that
   fixes the bug.**
4. **Session 4 — Phase 4 + 5.** Status-line error display. GUI File menu
   uses the same `project_apply::*` entry points as MCP. Final dead-code
   removal.
5. **Session 5 — Phase 6.** TODO + history cleanup, docs sweep, version
   bump if applicable.

Each session is self-contained and ships green; if the work is paused
between sessions the tree is in a working state.

---

## 9. Future direction (not in scope)

Once this lands, the same pattern can absorb the other `pending_*` queues:

- `pending_patch` → MCP applies patch to shared state directly; GUI
  refreshes on revision bump.
- `pending_awe_state` → already lives in shared `awe_state`; MCP writes
  directly; remove the queue.
- `pending_auto_layout` → already an `AtomicBool`; GUI polls; can stay
  as-is, or migrate to a `revision` style for symmetry.

**Co-locate scattered shared Arcs inside `McpSharedState`.** Today
`session`, `sample_library`, and `song` are sibling `Arc`s constructed in
`main.rs` and passed into `AppSynthBridge::new` as separate parameters.
Functionally fine; cosmetically `ProjectApplyContext` ends up holding
6–7 individual Arcs. Pulling them inside `McpSharedState` (or renaming
it to `SharedRuntime`) shrinks the context to a single Arc and prepares
for the next step.

Longer term: unify scattered shared state into
`Arc<RwLock<ProjectRuntimeState>>` containing `Song`, sample library,
AWE state, AWE description, author, and instrument descriptors. Atomic
project-swap becomes a single write-lock instead of N coordinated
locks. Not required by this plan, but it's the natural endpoint.
