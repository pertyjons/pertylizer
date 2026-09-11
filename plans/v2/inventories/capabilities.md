# Capability and Reachability Inventory

| Field         | Value      |
|---------------|------------|
| Status        | Active     |
| Phase         | 00B        |
| Last reviewed | 2026-09-12 (MCP discovery subset only) |

This ledger covers every shipped or externally consumed capability and assigns it a deliberate V2 disposition.

## Known baseline seeds

These counts come from the architecture audit recorded in the master plan. They are discovery seeds, not completeness
assertions or frozen product limits.

| Surface                       | Known baseline | Pass-1 count at `dd69b657` |
|-------------------------------|---------------:|---------------------------:|
| MCP tools                     |            219 |                        219 |
| Module types                  |             75 |                         75 |
| Programmatic built-in patches |             68 |                         68 |
| Group templates               |             12 |                         12 |

All four seeds reproduce exactly. Per inventory rule 2 that is **not** evidence of coverage — it only means the seeds
were counted the same way twice. The surfaces below that carry no seed (GUI, CLI, engine protocol, public Rust API,
formats, OSC) are the ones where omission is actually likely.

The complete audit must also discover GUI actions, menus, shortcuts, dialogs, background jobs, CLI entry points, public
Rust exports, formats, schemas, examples, OSC, the standalone visualizer, configuration, and tested-only or exported
subsystems.

## Allowed dispositions

- `Migrate`
- `Replace`
- `Remove`
- `Defer`
- `Compatibility adapter`

## Ledger

Entries use `CAP-NNNN` identifiers. Next free identifier: `CAP-0510`.

Passes 1 and 2 were a **surface census**; pass 3 added the per-item enumeration the master plan requires.
The [2026-09-09 project-action inspection](#project-actions-2026-09-09) assigns `Migrate` to seven existing
capabilities: `CAP-0048` through `CAP-0053`, and `CAP-0055`. The
[2026-09-11 CLI inspection](#cli-entry-points-2026-09-11) assigns `Migrate` to `CAP-0040` through
`CAP-0043` and the newly enumerated `CAP-0509`. The
[MCP project-operation inspection](#mcp-project-operations-2026-09-11) adds `CAP-0172`, `CAP-0173`,
`CAP-0205` and `CAP-0206`. The subsequent
[cleanup, lint and example-patch inspection](#mcp-cleanup-lint-and-example-patches-2026-09-12) adds
`CAP-0084`, `CAP-0153`, `CAP-0156`, `CAP-0171` and `CAP-0178`. The
[discovery inspection](#mcp-discovery-2026-09-12) adds `CAP-0140`, `CAP-0143`, `CAP-0162`, `CAP-0167` and
`CAP-0208`. Other dispositions remain open.

**Status rule.** The [register vocabulary](README.md) defines `Classified` as required fields *and* disposition filled
with supporting evidence. The twenty-six inspected rows meet that classification threshold, not migration verification.
Entries whose disposition is open stay `Discovered` or `Investigating`. `Verified` requires the named migration
checks to pass; source inspection alone does not establish implemented V2 behavior. P00B-T002 remains incomplete.

### MCP protocol surface (219 tools)

| ID       | Surface | Capability                                                                                                                                                           | Reachable from                                                       | Disposition | V2 owner/replacement | Evidence                                                                                                                                                                                                                                        | Status        |
|----------|---------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------|-------------|----------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------|
| CAP-0001 | MCP     | Sequencer tools — 63 (`server/tools/sequencer.rs`): patterns, notes, tracks, arrangement, tempo map, transport, sections, note/mod graphs                            | `synth` server over HTTP `127.0.0.1:9850/mcp` and `--headless` stdio |             |                      |                                                                                                                                                                                                                                                 | Discovered    |
| CAP-0002 | MCP     | Instrument tools — 34 (`instruments.rs`): create/build/delete, modules, connections, parameters, patches, schema                                                     | Same                                                                 |             |                      |                                                                                                                                                                                                                                                 | Discovered    |
| CAP-0003 | MCP     | Analysis tools — 32 (`analysis.rs`): harmony, mix bus, sections, spectra, spectrograms, envelopes, groove, masking, tension, motifs                                  | Same                                                                 |             |                      | Offline analyzers read source, not note-processor expansion — a known documented seam                                                                                                                                                           | Investigating |
| CAP-0004 | MCP     | Mixing tools — 30 (`mixing.rs`): return buses, sends, master/return effects, bypass, solo, color, description, sidechain                                             | Same                                                                 |             |                      |                                                                                                                                                                                                                                                 | Discovered    |
| CAP-0005 | MCP     | Discovery tools — 19 (`discovery.rs`): module catalog, type info, search, port types, connection check, YAMS reference, engine status                                | Same                                                                 |             |                      |                                                                                                                                                                                                                                                 | Discovered    |
| CAP-0006 | MCP     | Sample tools — 16 (`samples.rs`): import, export, crop, loop, normalize, reverse, trim, root note, duplicate, delete                                                 | Same                                                                 |             |                      |                                                                                                                                                                                                                                                 | Discovered    |
| CAP-0007 | MCP     | Automation tools — 12 (`automation.rs`): points, lanes, copy, scale, offset, simplify, clear, summary                                                                | Same                                                                 |             |                      |                                                                                                                                                                                                                                                 | Discovered    |
| CAP-0008 | MCP     | Audio-input tools — 7 (`audio_input.rs`): device list/select, monitoring, recording                                                                                  | Same                                                                 |             |                      | Device lifecycle is ADR-0036                                                                                                                                                                                                                    | Discovered    |
| CAP-0009 | MCP     | Project tools — 5 (`project.rs`): new, load, save, save patch, optimize; lint is CAP-0153 in discovery                                                                                                  | Same                                                                 |             |                      | Save path is `STATE-0027`/`STATE-0031` sensitive                                                                                                                                                                                                | Investigating |
| CAP-0010 | MCP     | `batch_execute` — 1 (`batch.rs`) plus the `dispatch_tools!` macro that routes every tool through three reply shapes (text / typed payload / action)                  | Same                                                                 |             |                      | Has a dispatch-guard test                                                                                                                                                                                                                       | Discovered    |
| CAP-0011 | MCP | Behavior annotations — of 219 tools: **71** `read_only_hint = true`, **97** `destructive_hint = false`, **51** `destructive_hint = true`. 71 + 97 + 51 = 219, so coverage is exactly complete and every tool carries precisely one behavior annotation | Tool metadata | | | Counted after excluding commented-out occurrences — a raw `rg` returns 98 `false` because `server/tools/batch.rs:35` explains in a comment why `batch_execute` is *not* `destructive_hint = false`. The 51 destructive tools are the set a V2 authorization policy would gate first (ADR-0029) | Discovered |
| CAP-0012 | MCP     | Resource completions and closed-set schema enums                                                                                                                     | MCP protocol                                                         |             |                      | Landed in `67b5afa1`                                                                                                                                                                                                                            | Discovered    |
| CAP-0013 | MCP     | Structured output + handler-stated verdict on every tool                                                                                                             | MCP protocol                                                         |             |                      | Landed in `9ccbfe35`                                                                                                                                                                                                                            | Discovered    |

### Engine protocol surface

| ID       | Surface | Capability                                                                                                                                                                       | Reachable from                                 | Disposition | V2 owner/replacement | Evidence                                                     | Status        |
|----------|---------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------|-------------|----------------------|--------------------------------------------------------------|---------------|
| CAP-0014 | Engine  | `EngineCommand` — 76 variants (`crates/synth_engine/src/commands.rs:250`)                                                                                                        | GUI, MCP bridge, render CLI                    |             |                      | Every variant needs an individual row in pass 2              | Discovered    |
| CAP-0015 | Engine  | `EngineEvent` — 14 variants (same file) incl. `PeakMeter`, `RmsMeter`, `VoiceCount`, `CpuUsage`, `BufferUnderrun`, `KeyRangeLearned`, `RecordingPreview`, `RecordedNotesFlushed` | GUI, MCP, OSC sender                           |             |                      |                                                              | Discovered    |
| CAP-0016 | Engine  | Prioritized event channel with per-priority drop counters                                                                                                                        | Engine → frontends                             |             |                      | See `LIMIT-0013`                                             | Discovered    |
| CAP-0017 | Engine  | Multi-client hub (`hub.rs`) — per-client event buffers, `ClientId`                                                                                                               | Public Rust API; no in-workspace caller; external use is not observable from this repository | Proposed removal from initial V2; Phase 10E decides tested local-only removal or a service successor | Phase 10E service/public-facade contract | Proposed ADR-0039; EVD-0005 supports only the no-workspace-caller claim | Investigating |
| CAP-0018 | Engine  | `CommandSync` drop counter and save barrier                                                                                                                                      | All save paths                                 |             |                      | Merged `fb7b710b`; see `LIMIT-0012`                          | Discovered    |
| CAP-0019 | Engine  | Voice allocator — allocation modes, stealing strategies, unison, key ranges                                                                                                      | GUI, MCP, project                              |             |                      |                                                              | Discovered    |
| CAP-0020 | Engine  | Recording engine — held/recorded note buffers, take flush, preview                                                                                                               | GUI, MCP audio-input tools                     |             |                      | Take semantics are ADR-0024                                  | Discovered    |
| CAP-0021 | Engine  | Real-time allocation guard (`rt_alloc_guard.rs`)                                                                                                                                 | Debug/test builds                              |             |                      |                                                              | Discovered    |
| CAP-0022 | Engine  | CPU tracker and per-module profiling (`cpu_tracker.rs`, `rt-profiling` feature)                                                                                                  | Opt-in feature                                 |             |                      |                                                              | Discovered    |

### Module, patch, and template catalog

| ID       | Surface   | Capability                                                                                              | Reachable from                                                                     | Disposition | V2 owner/replacement | Evidence                                                          | Status        |
|----------|-----------|---------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------|-------------|----------------------|-------------------------------------------------------------------|---------------|
| CAP-0023 | Modules   | 75 module types (`ModuleType`, `crates/synth_core/src/params/mod.rs`)                                   | Patch editor, MCP `add_module`, project load                                       |             |                      | Matches the seed exactly; per-type rows are pass-2 work           | Discovered    |
| CAP-0024 | Modules   | Mod Matrix — 16 slots per voice, generic `set_mod_offset` on all 40 voice modules                       | Patch editor, MCP                                                                  |             |                      |                                                                   | Discovered    |
| CAP-0025 | Modules   | Note processors (NP1–NP7) and the note-graph pool                                                       | Pattern editor, MCP                                                                |             |                      | Documented design debts: `map_pitch` 1→N seam, last-one-wins rack | Investigating |
| CAP-0026 | Modules   | Mod Grid / Note Grid node graphs, 32 nodes each                                                         | GUI grid views, MCP                                                                |             |                      |                                                                   | Discovered    |
| CAP-0027 | Patches   | 68 programmatic built-in patches (`crates/pertylizer/src/patches/`)                                     | GUI browser, MCP `list_example_patches`/`load_example_patch`/`apply_example_patch` |             |                      | Matches the seed                                                  | Discovered    |
| CAP-0028 | Templates | 12 built-in group templates in 4 categories (voice 3, effect 3, utility 4, tutorial 2)                  | Patch editor group menu                                                            |             |                      | Matches the seed                                                  | Discovered    |
| CAP-0029 | Templates | User group templates loaded from disk                                                                   | `GroupTemplateManager`                                                             |             |                      | See `STATE-0055`                                                  | Discovered    |
| CAP-0030 | Modules   | YAMS script runtimes — control-rate, audio-rate `AudioScript`, note-script transform, Mod Matrix script | Patch editor ƒx editor, MCP `set_mod_matrix_script`, `set_note_graph_script`       |             |                      | Budgets are `LIMIT-0032`..`LIMIT-0042`                            | Discovered    |

### GUI surface

| ID       | Surface | Capability                                                                                                                                               | Reachable from                           | Disposition | V2 owner/replacement | Evidence                                                                                                                     | Status        |
|----------|---------|----------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------|-------------|----------------------|------------------------------------------------------------------------------------------------------------------------------|---------------|
| CAP-0031 | GUI     | Patch editor + node canvas (`egui::Scene`), auto-layout, groups, exposed ports                                                                           | `gui-egui` feature (default)             |             |                      | Owner of `STATE-0027`/`STATE-0031`/`STATE-0032`                                                                              | Investigating |
| CAP-0032 | GUI     | Sequencer views — arrangement, piano roll, pattern view, tracker view                                                                                    | Default build                            |             |                      |                                                                                                                              | Discovered    |
| CAP-0033 | GUI     | Mixer view, master-effects view, meters panels                                                                                                           | Default build                            |             |                      |                                                                                                                              | Discovered    |
| CAP-0034 | GUI     | Mod Grid view, Note Grid view, script editor                                                                                                             | Default build                            |             |                      |                                                                                                                              | Discovered    |
| CAP-0035 | GUI     | Sample view, instrument rack, list panel, module panel, welcome view, activity log view                                                                  | Default build                            |             |                      |                                                                                                                              | Discovered    |
| CAP-0036 | GUI     | Dialogs and file-dialog workflows (`gui/dialogs.rs`), export dialog                                                                                      | Default build                            |             |                      | Dialog inventory not enumerated per dialog                                                                                   | Investigating |
| CAP-0037 | GUI     | Keyboard input, on-screen keyboard, clipboard, and the `InputGate` that stops the computer-keyboard piano from eating modified keys and text-field input | Default build                            |             |                      | The gate exists because bare-letter note keys used to fire on the way to `Ctrl+S`/`Ctrl+Z` and while typing into text fields | Discovered    |
| CAP-0038 | GUI     | Theme presets and bundled monospace fonts                                                                                                                | Default build; persisted in `STATE-0048` |             |                      |                                                                                                                              | Discovered    |
| CAP-0039 | GUI     | AccessKit inspection surface — `egui-inspection` feature, `EGUI_INSPECTION=1`, port 5719                                                                 | Opt-in, non-default                      |             |                      | Custom painter widgets emit no label                                                                                         | Investigating |

### CLI, service, and external consumers

| ID       | Surface    | Capability                                                                                                                                                                         | Reachable from               | Disposition | V2 owner/replacement | Evidence                                                                                 | Status        |
|----------|------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------|-------------|----------------------|------------------------------------------------------------------------------------------|---------------|
| CAP-0040 | CLI | `pertylizer` without a mode selector launches the GUI | `main::run_gui` when `gui-egui` is enabled; absent GUI feature returns an error | Migrate | Phase 11 GUI adapter over Application Core; Phase 9 audio host; Phase 10E service/configuration adapters | [GUI launch trace](#gui-launch-and-osc-switch); backend-start success is not established by parsing | Classified |
| CAP-0041 | CLI | `pertylizer --headless` serves MCP on stdio using a null audio backend | `mcp` feature: `main::run_headless_mcp` → `synth_mcp::serve_stdio` | Migrate | Phase 10E MCP/service adapter over Application Core and runtime session; wire conformance belongs to that adapter; ADR-0029/0030 are proposed authorization and public-facade topics | [Stdio trace](#headless-stdio); individual tools remain separately inventoried | Classified |
| CAP-0042 | CLI | `pertylizer render` — saved project/bundle/patch to WAV with mix selection and versioned receipt | Ungated `Command::Render` → `render_project` → `render::run_render_command`; ten declared options, excluding generated help | Migrate | Phase 10D Project I/O; Phase 10B revision-pinned render jobs under ADR-0028; Phase 10E CLI adapter over Sound Core rendering | [Render trace](#render-command); existing V1 protocol version 1 is unchanged; [open CLI work](../../TODO.md#56-headless-render-cli--open-follow-ups) | Classified |
| CAP-0043 | CLI | `--no-osc` suppresses GUI OSC telemetry startup for this invocation | Top-level argument when `osc` is enabled; read only by `run_gui` | Migrate | Phase 10E host configuration/telemetry adapter; Phase 11 startup integration; ADR-0029 owns durable configuration policy | [GUI launch trace](#gui-launch-and-osc-switch); not a project mutation or an MCP-disable flag | Classified |
| CAP-0509 | CLI | `pertylizer compare` — measure two existing WAVs and emit a versioned JSON report, without a parity verdict | Ungated `Command::Compare` → `compare_renders` → `compare::run_compare_command`; four declared options, excluding generated help | Migrate | Retained Phase 0A comparison utility; Phase 10E CLI/analysis adapter; any later render-job integration remains ADR-0028's Phase 10B work | [Compare trace](#compare-command); no project lowering or rendering occurs in this command | Classified |
| CAP-0044 | Protocol   | OSC telemetry — 19 addresses under `/synth/*` and `/viz/*` (meta, RMS, peak, FFT, centroid, flux, note on/off, CC, transport, voice count, CPU, event drops, viz ping/pong/camera) | `osc` feature (default), UDP |             |                      | External standalone visualizer consumes `/viz/*`; it is not in this repository           | Investigating |
| CAP-0045 | Formats    | `.ptz` project (JSON), `.ptz.zip` bundle, `.json` patch, `.json` group template, `settings.json`, recovery snapshots                                                               | File dialogs, CLI, MCP       |             |                      | Three committed JSON Schemas: `project`, `patch`, `bundle-metadata`                      | Investigating |
| CAP-0046 | Public API | **23** `pub mod` in `crates/pertylizer/src/lib.rs` plus 11 further workspace crates, all with public surfaces | Rust consumers | | | Facade scope is ADR-0030; the planned runtime library is `plans/game-runtime-library.md` | Investigating |
| CAP-0047 | Build      | Cargo features — `gui-egui`, `mcp`, `osc` (default), `rt-profiling`, `egui-inspection` (opt-in); MSRV 1.98; CI checks `--no-default-features` and `--all-features`                 | Build matrix                 |             |                      | Supported matrix is ADR-0031                                                             | Discovered    |

### GUI actions (pass 2)

Pass 1 listed GUI *views*; pass 2 splits out the actions that are dispatched application-wide, which is the set a V2
frontend must reproduce identically. View-local editing (note entry, module selection, dragging) deliberately stays with
the view that owns it and is not an app-level capability.

| ID       | Surface      | Capability                                                                                       | Reachable from                                                                     | Disposition | V2 owner/replacement | Evidence                                                      | Status        |
|----------|--------------|--------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------|-------------|----------------------|---------------------------------------------------------------|---------------|
| CAP-0048 | GUI action | `New` project — Cmd/Ctrl+N | Default `gui-egui` build: File menu and `handle_app_shortcuts` → `request_new_project` | Migrate | Application Core document lifecycle over Project Core; core lifecycle in 10A–10C; frontend confirmation in 11 | [New/Open trace](#new-and-open); `STATE-0058`, `STATE-0059` | Classified |
| CAP-0049 | GUI action | `Open` project — Cmd/Ctrl+O, including smart-open dispatch | Default `gui-egui` build: File menu and shortcut → `request_open_project`; Home Open also reaches the same file-dialog mode | Migrate | Project I/O decode/convert/validate; Application Core open; core work in 10A–10D; frontend path picker in 11 | [New/Open trace](#new-and-open); current-project format conversion belongs to 10D | Classified |
| CAP-0050 | GUI action | `Save` — Cmd/Ctrl+S; requests a path for an untitled project | Default `gui-egui` build: File menu and shortcut → `save_current_project` → `save_current_project_outcome` | Migrate | Application Core save coordinator over a canonical Project Core snapshot; Project I/O writer (10C/10D); GUI adapter (11) | [Save trace](#save-and-save-as); `STATE-0027`, `STATE-0031`, `STATE-0032`, `STATE-0058` | Classified |
| CAP-0051 | GUI action | `Save As` — Shift+Cmd/Ctrl+S; select a new output path | Default `gui-egui` build: File menu and shortcut → `open_save_project_as_dialog` → SaveProject file-dialog result | Migrate | Application Core save coordination and recovery association; Project I/O (10C/10D); frontend path picker (11) | [Save trace](#save-and-save-as); `STATE-0057` | Classified |
| CAP-0052 | GUI action | `Undo` — Cmd/Ctrl+Z; reverse the last recorded edit | Default `gui-egui` build: Edit menu (enabled by `can_undo`) and shortcut → `execute_undo` | Migrate | Application Core history over canonical operations (10B/10C); representation remains ADR-0015; GUI adapter in 11 | [History trace](#undo-and-redo); `STATE-0058`, `STATE-0059`, `IDN-0027` | Classified |
| CAP-0053 | GUI action | `Redo` — Shift+Cmd/Ctrl+Z; reapply the last undone edit | Default `gui-egui` build: Edit menu (enabled by `can_redo`) and shortcut → `execute_redo` | Migrate | Application Core history over canonical operations (10B/10C); representation remains ADR-0015; GUI adapter in 11 | [History trace](#undo-and-redo); `STATE-0058`, `STATE-0059` | Classified |
| CAP-0054 | GUI action | `Toggle playback` — `Space`, gated so it never fires while text is focused | Default GUI build: application shortcut dispatcher → `toggle_playback` |  |  |  | Discovered |
| CAP-0055 | GUI workflow | Startup recovery offer: recover or discard a retained unsaved project snapshot | Default `gui-egui` build: `SynthApp::new` → `check_for_recoverable_work`; `show_dialogs` draws the pending offer first | Migrate | Application Core recovery/save coordination (10C), Project I/O assets (10D), runtime-session scheduling; frontend prompt in 11 | [Recovery trace](#startup-recovery); `STATE-0054`, `STATE-0057`; recording-take commit is separate (ADR-0024) | Classified |

`AppShortcut::ALL` enumerates seven application shortcuts at the inspected revision. The File menu uses
`AppShortcut` bindings for New/Open/Save/Save As; the Edit menu invokes the same undo/redo handlers but hardcodes
its shortcut labels. The table is not an exhaustive GUI-action census: view-local commands, other menu entries
and the startup workflow have separate entry points. `CAP-0054` is outside this project-action slice.

## Per-item enumerations

The master plan requires this ledger to name **every** `EngineCommand` and `EngineEvent` variant, every MCP tool with
its read/mutate behavior, and every module type, built-in patch, and group template — not a family count. The rows
below are that enumeration, generated from source at `dd69b657` by the method recorded in the pass-3 audit row, so a
capability added or removed later shows up as a diff rather than as a changed total.

`CAP-0001`..`CAP-0010`, `CAP-0014`, `CAP-0015`, `CAP-0023`, and `CAP-0027` remain at their stable identifiers as
**rollup rows**: they describe a surface, carry no disposition of their own, and are not counted as capability entries.
The authoritative per-capability entries are below. Fourteen MCP entries are now classified by the
[project-operation inspection](#mcp-project-operations-2026-09-11),
[cleanup, lint and example-patch inspection](#mcp-cleanup-lint-and-example-patches-2026-09-12) and
[discovery inspection](#mcp-discovery-2026-09-12), departing from
the generated rows' original `Discovered` status. The other generated entries still lack a disposition;
reachability alone cannot make them `Classified`. Evidence for each classified MCP entry is linked from its
`Reachable from` cell.

### MCP tools (219)

`Behavior` is the tool's own annotation: `read` = `read_only_hint = true`; `mutating` = `destructive_hint = false`;
`destructive` = `destructive_hint = true`. Every tool carries exactly one, so this column is complete by construction.

| ID | Surface | Tool | Module | Behavior | Reachable from | Disposition | V2 owner/replacement | Status |
|----|---------|------|--------|----------|----------------|-------------|----------------------|--------|
| CAP-0056 | MCP | `add_automation_points` | `automation.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0057 | MCP | `add_master_effect` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0058 | MCP | `add_mod_graph_node` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0059 | MCP | `add_module` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0060 | MCP | `add_note` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0061 | MCP | `add_note_graph_module` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0062 | MCP | `add_return_effect` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0063 | MCP | `analyze_arrangement` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0064 | MCP | `analyze_bass_drum_lock` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0065 | MCP | `analyze_drum_groove` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0066 | MCP | `analyze_form_map` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0067 | MCP | `analyze_harmonic_function` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0068 | MCP | `analyze_harmony` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0069 | MCP | `analyze_hook_strength` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0070 | MCP | `analyze_instrument_range` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0071 | MCP | `analyze_masking_matrix` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0072 | MCP | `analyze_master_chain` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0073 | MCP | `analyze_mix_bus` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0074 | MCP | `analyze_note` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0075 | MCP | `analyze_pattern` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0076 | MCP | `analyze_return_busses` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0077 | MCP | `analyze_sample_spectrogram` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0078 | MCP | `analyze_sample_spectrum` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0079 | MCP | `analyze_section` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0080 | MCP | `analyze_spectrogram` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0081 | MCP | `analyze_spectrum` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0082 | MCP | `analyze_tension_curve` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0083 | MCP | `analyze_velocity_response` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0084 | MCP | `apply_example_patch` | `instruments.rs` | destructive | Default MCP GUI HTTP and `--headless` stdio → `InstrumentBuildBridge::apply_example_patch`; [source trace](#mcp-direct-example-patch-application) | Migrate | Phase 10A canonical instrument/catalog content; 10B replacement operation; 10C history/dirty state; 10E MCP adapter; 11 GUI reconciliation | Classified |
| CAP-0085 | MCP | `assign_mod_graph` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0086 | MCP | `assign_sample_to_module` | `samples.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0087 | MCP | `auto_gain_stage` | `analysis.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0088 | MCP | `auto_layout` | `instruments.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0089 | MCP | `batch_execute` | `batch.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0090 | MCP | `build_instrument` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0091 | MCP | `check_connection` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0092 | MCP | `clear_automation_lane` | `automation.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0093 | MCP | `clear_graph` | `instruments.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0094 | MCP | `clear_pattern` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0095 | MCP | `clear_transport_loop` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0096 | MCP | `compare_envelopes` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0097 | MCP | `compare_mix_before_after` | `instruments.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0098 | MCP | `compare_spectra` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0099 | MCP | `connect` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0100 | MCP | `connect_mod_graph` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0101 | MCP | `connect_note_graph` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0102 | MCP | `copy_automation_lane` | `automation.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0103 | MCP | `create_chord_progression_pattern` | `analysis.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0104 | MCP | `create_instrument` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0105 | MCP | `create_mod_graph` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0106 | MCP | `create_note_graph` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0107 | MCP | `create_pattern` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0108 | MCP | `create_return_bus` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0109 | MCP | `create_track` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0110 | MCP | `delete_instrument` | `instruments.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0111 | MCP | `delete_mod_graph` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0112 | MCP | `delete_note_graph` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0113 | MCP | `delete_pattern` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0114 | MCP | `delete_return_bus` | `mixing.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0115 | MCP | `delete_sample` | `samples.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0116 | MCP | `delete_track` | `mixing.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0117 | MCP | `disconnect` | `instruments.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0118 | MCP | `disconnect_mod_graph` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0119 | MCP | `duplicate_mod_graph` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0120 | MCP | `duplicate_note_graph` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0121 | MCP | `duplicate_pattern` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0122 | MCP | `duplicate_sample` | `samples.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0123 | MCP | `export_sample` | `samples.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0124 | MCP | `find_motifs` | `instruments.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0125 | MCP | `freeze_pattern` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0126 | MCP | `generate_chord` | `analysis.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0127 | MCP | `get_automation_points` | `automation.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0128 | MCP | `get_automation_summary` | `automation.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0129 | MCP | `get_connections` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0130 | MCP | `get_engine_status` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0131 | MCP | `get_graph_diagnostics` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0132 | MCP | `get_input_state` | `audio_input.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0133 | MCP | `get_instrument_automation_targets` | `automation.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0134 | MCP | `get_instrument_info` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0135 | MCP | `get_instrument_profiles` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0136 | MCP | `get_master_volume` | `mixing.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0137 | MCP | `get_mod_graph` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0138 | MCP | `get_mod_matrix_routings` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0139 | MCP | `get_module_info` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0140 | MCP | `get_module_type_info` | `discovery.rs` | read | Default MCP GUI HTTP and `--headless` stdio → `DiscoveryBridge::get_module_type_info`; [source trace](#mcp-module-type-detail) | Migrate | Phase 5 node/parameter declarations and metadata; 10E MCP detail adapter | Classified |
| CAP-0141 | MCP | `get_note_graph` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0142 | MCP | `get_parameter` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0143 | MCP | `get_project_schema` | `discovery.rs` | read | Default MCP GUI HTTP and `--headless` stdio → `InstrumentBridge::get_project_schema`; [source trace](#mcp-project-schema-discovery) | Migrate | Phase 10D versioned format/schema artifacts; 10E MCP schema adapter | Classified |
| CAP-0144 | MCP | `get_sample_info` | `samples.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0145 | MCP | `get_sampler_state` | `samples.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0146 | MCP | `get_song_info` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0147 | MCP | `get_tempo_map` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0148 | MCP | `get_ui_snapshot` | `instruments.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0149 | MCP | `get_version` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0150 | MCP | `get_yams_reference` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0151 | MCP | `import_sample` | `samples.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0152 | MCP | `insert_module_between` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0153 | MCP | `lint_project` | `discovery.rs` | read | Default MCP GUI HTTP and `--headless` stdio → `InstrumentBridge::lint_project`; [source trace](#mcp-project-lint) | Migrate | Phase 10A canonical validation and graph diagnostics; 10E MCP read adapter | Classified |
| CAP-0154 | MCP | `list_arrangement` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0155 | MCP | `list_automation_lanes` | `automation.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0156 | MCP | `list_example_patches` | `instruments.rs` | read | Default MCP GUI HTTP and `--headless` stdio → `InstrumentBridge::list_example_patches`; [source trace](#mcp-example-patch-discovery) | Migrate | Phase 10A catalog content; 10E MCP discovery adapter | Classified |
| CAP-0157 | MCP | `list_input_devices` | `audio_input.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0158 | MCP | `list_instruments` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0159 | MCP | `list_master_effects` | `mixing.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0160 | MCP | `list_mod_graphs` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0161 | MCP | `list_mod_targets` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0162 | MCP | `list_module_types` | `discovery.rs` | read | Default MCP GUI HTTP and `--headless` stdio → `InstrumentBridge::list_module_types_brief`; [source trace](#mcp-module-type-listing) | Migrate | Phase 5 node declarations and derived catalog; 10E MCP discovery adapter | Classified |
| CAP-0163 | MCP | `list_modules` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0164 | MCP | `list_note_graphs` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0165 | MCP | `list_notes` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0166 | MCP | `list_patterns` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0167 | MCP | `list_port_types` | `discovery.rs` | read | Default MCP GUI HTTP and `--headless` stdio → `SynthMcpServer::list_port_types`; [source trace](#mcp-port-type-discovery) | Migrate | Phase 5 port declarations and compiler compatibility; 10E MCP discovery adapter | Classified |
| CAP-0168 | MCP | `list_return_busses` | `mixing.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0169 | MCP | `list_samples` | `samples.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0170 | MCP | `list_tracks` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0171 | MCP | `load_example_patch` | `instruments.rs` | destructive | Default MCP GUI HTTP and `--headless` stdio → `InstrumentBridge::load_example_patch`; [source trace](#mcp-example-patch-load-and-gui-consumer) | Migrate | Phase 10A canonical instrument/catalog content; 10B creation operation; 10C history/dirty state; 10E MCP adapter; 11 targeted GUI metadata | Classified |
| CAP-0172 | MCP | `load_project` | `project.rs` | destructive | Default MCP GUI HTTP and `--headless` stdio → `ProjectBridge::load_project`; [source trace](#mcp-project-load) | Migrate | Phase 10D Project I/O decode/convert/validate and assets; 10B application lifecycle; 10C saved-state coordination; 10E MCP adapter; 11 GUI refresh | Classified |
| CAP-0173 | MCP | `new_project` | `project.rs` | destructive | Default MCP GUI HTTP and `--headless` stdio → `ProjectBridge::new_project`; [source trace](#mcp-project-reset) | Migrate | Phase 10A canonical empty document; 10B application lifecycle; 10C history/saved-state coordination; 10E MCP adapter; 11 GUI refresh | Classified |
| CAP-0174 | MCP | `normalize_sample` | `samples.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0175 | MCP | `note_off` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0176 | MCP | `note_on` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0177 | MCP | `offset_automation_lane` | `automation.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0178 | MCP | `optimize_project` | `project.rs` | destructive | Default MCP GUI HTTP and `--headless` stdio → `ProjectBridge::optimize_project`; [source trace](#mcp-project-cleanup) | Migrate | Phase 10A document references; 10B cleanup operation; 10C history/dirty state; 10D asset reachability; 10E MCP adapter | Classified |
| CAP-0179 | MCP | `place_pattern` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0180 | MCP | `preview_note` | `analysis.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0181 | MCP | `quantize_notes_to_grid` | `analysis.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0182 | MCP | `quantize_notes_to_scale` | `analysis.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0183 | MCP | `rebuild_instrument_preserve_automation` | `automation.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0184 | MCP | `remove_automation_points` | `automation.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0185 | MCP | `remove_master_effect` | `mixing.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0186 | MCP | `remove_mod_graph_node` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0187 | MCP | `remove_module` | `instruments.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0188 | MCP | `remove_note` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0189 | MCP | `remove_note_graph_module` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0190 | MCP | `remove_placement` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0191 | MCP | `remove_return_effect` | `mixing.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0192 | MCP | `remove_return_send` | `mixing.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0193 | MCP | `remove_tempo_at` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0194 | MCP | `remove_track_send` | `mixing.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0195 | MCP | `rename_instrument` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0196 | MCP | `rename_pattern` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0197 | MCP | `rename_return_bus` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0198 | MCP | `rename_sample` | `samples.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0199 | MCP | `rename_track` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0200 | MCP | `render_to_wav` | `analysis.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0201 | MCP | `reorder_master_effect` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0202 | MCP | `reorder_return_effect` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0203 | MCP | `replace_notes` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0204 | MCP | `reverse_sample` | `samples.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0205 | MCP | `save_patch` | `project.rs` | destructive | Default MCP GUI HTTP and `--headless` stdio → `ProjectBridge::save_patch`; [source trace](#mcp-patch-export) | Migrate | Phase 10A canonical instrument content; 10B export operation; 10D patch format/assets; 10E MCP adapter | Classified |
| CAP-0206 | MCP | `save_project` | `project.rs` | destructive | Default MCP GUI HTTP and `--headless` stdio → `ProjectBridge::save_project`; [source trace](#mcp-project-save) | Migrate | Phase 10A canonical snapshot; 10B operation results; 10C save coordination; 10D writer/assets; 10E MCP adapter; 11 GUI save integration | Classified |
| CAP-0207 | MCP | `scale_automation_lane` | `automation.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0208 | MCP | `search_modules` | `discovery.rs` | read | Default MCP GUI HTTP and `--headless` stdio → `DiscoveryBridge::search_modules`; [source trace](#mcp-module-search) | Migrate | Phase 5 node declarations and derived catalog; 10E MCP search/filter adapter | Classified |
| CAP-0209 | MCP | `seq_play` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0210 | MCP | `seq_seek` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0211 | MCP | `seq_stop` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0212 | MCP | `set_allocator_config` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0213 | MCP | `set_input_device` | `audio_input.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0214 | MCP | `set_instrument_category` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0215 | MCP | `set_instrument_color` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0216 | MCP | `set_instrument_description` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0217 | MCP | `set_instrument_midi_channel` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0218 | MCP | `set_instrument_mixer` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0219 | MCP | `set_master_effect_enabled` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0220 | MCP | `set_master_effect_parameter` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0221 | MCP | `set_master_volume` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0222 | MCP | `set_mod_graph_metadata` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0223 | MCP | `set_mod_graph_node` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0224 | MCP | `set_mod_graph_scope` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0225 | MCP | `set_mod_matrix_script` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0226 | MCP | `set_module_description` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0227 | MCP | `set_mseg_segments` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0228 | MCP | `set_note_graph_metadata` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0229 | MCP | `set_note_graph_module` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0230 | MCP | `set_note_graph_script` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0231 | MCP | `set_note_note_graph` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0232 | MCP | `set_note_ornament` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0233 | MCP | `set_parameter` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0234 | MCP | `set_patch_color` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0235 | MCP | `set_patch_description` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0236 | MCP | `set_pattern_description` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0237 | MCP | `set_pattern_length` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0238 | MCP | `set_pattern_note_graph` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0239 | MCP | `set_return_bus_color` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0240 | MCP | `set_return_bus_description` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0241 | MCP | `set_return_bus_mixer` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0242 | MCP | `set_return_effect_enabled` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0243 | MCP | `set_return_effect_parameter` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0244 | MCP | `set_return_send` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0245 | MCP | `set_sample_crop` | `samples.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0246 | MCP | `set_sample_description` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0247 | MCP | `set_sample_loop` | `samples.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0248 | MCP | `set_sample_root_note` | `samples.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0249 | MCP | `set_sampler_parameter` | `samples.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0250 | MCP | `set_sidechain_source` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0251 | MCP | `set_song` | `sequencer.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0252 | MCP | `set_song_author` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0253 | MCP | `set_song_description` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0254 | MCP | `set_song_name` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0255 | MCP | `set_song_tempo` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0256 | MCP | `set_song_time_signature` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0257 | MCP | `set_tempo_at` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0258 | MCP | `set_track_color` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0259 | MCP | `set_track_description` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0260 | MCP | `set_track_instrument` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0261 | MCP | `set_track_mixer` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0262 | MCP | `set_track_send` | `mixing.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0263 | MCP | `set_transport_loop` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0264 | MCP | `simplify_automation` | `automation.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0265 | MCP | `start_monitoring` | `audio_input.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0266 | MCP | `start_recording` | `audio_input.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0267 | MCP | `stop_monitoring` | `audio_input.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0268 | MCP | `stop_recording` | `audio_input.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0269 | MCP | `suggest_music_fixes` | `analysis.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0270 | MCP | `transpose_notes` | `analysis.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0271 | MCP | `trim_sample_silence` | `samples.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0272 | MCP | `update_note` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0273 | MCP | `update_placement` | `sequencer.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0274 | MCP | `validate_instrument_audio` | `instruments.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |

### `EngineCommand` variants (76)

| ID | Surface | Capability | Reachable from | Disposition | V2 owner/replacement | Evidence | Status |
|----|---------|------------|----------------|-------------|----------------------|----------|--------|
| CAP-0275 | Engine | `EngineCommand::AddInstrument` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0276 | Engine | `EngineCommand::RemoveInstrument` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0277 | Engine | `EngineCommand::RenameInstrument` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0278 | Engine | `EngineCommand::SetInstrumentDescription` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0279 | Engine | `EngineCommand::SetPatchDescription` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0280 | Engine | `EngineCommand::SetInstrumentColor` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0281 | Engine | `EngineCommand::SetPatchColor` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0282 | Engine | `EngineCommand::SetModuleDescription` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0283 | Engine | `EngineCommand::SetSidechainSource` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0284 | Engine | `EngineCommand::SetInstrumentParameter` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0285 | Engine | `EngineCommand::SetInstrumentMidiChannel` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0286 | Engine | `EngineCommand::SetInstrumentEnabled` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0287 | Engine | `EngineCommand::SetInstrumentCategory` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0288 | Engine | `EngineCommand::SetInstrumentSolo` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0289 | Engine | `EngineCommand::CreateReturnBus` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0290 | Engine | `EngineCommand::RemoveReturnBus` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0291 | Engine | `EngineCommand::ClearReturnBusses` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0292 | Engine | `EngineCommand::AddReturnEffect` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0293 | Engine | `EngineCommand::RemoveReturnEffect` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0294 | Engine | `EngineCommand::SetReturnEffectParameter` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0295 | Engine | `EngineCommand::SetReturnEffectEnabled` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0296 | Engine | `EngineCommand::ReorderReturnEffect` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0297 | Engine | `EngineCommand::SetReturnEffectChainOrder` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0298 | Engine | `EngineCommand::ClearMasterEffects` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0299 | Engine | `EngineCommand::NoteOn` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0300 | Engine | `EngineCommand::NoteOff` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0301 | Engine | `EngineCommand::AllNotesOff` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0302 | Engine | `EngineCommand::ResetDsp` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0303 | Engine | `EngineCommand::PitchBend` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0304 | Engine | `EngineCommand::ModWheel` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0305 | Engine | `EngineCommand::ControlChange` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0306 | Engine | `EngineCommand::Aftertouch` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0307 | Engine | `EngineCommand::PolyAftertouch` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0308 | Engine | `EngineCommand::SetVoiceParameter` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0309 | Engine | `EngineCommand::SetModuleParameter` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0310 | Engine | `EngineCommand::SetModScript` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0311 | Engine | `EngineCommand::AddModuleInstance` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0312 | Engine | `EngineCommand::RemoveModule` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0313 | Engine | `EngineCommand::Connect` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0314 | Engine | `EngineCommand::Disconnect` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0315 | Engine | `EngineCommand::DisconnectAll` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0316 | Engine | `EngineCommand::SetTempo` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0317 | Engine | `EngineCommand::Play` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0318 | Engine | `EngineCommand::Stop` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0319 | Engine | `EngineCommand::Pause` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0320 | Engine | `EngineCommand::Rewind` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0321 | Engine | `EngineCommand::Seek` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0322 | Engine | `EngineCommand::SetLoop` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0323 | Engine | `EngineCommand::SetRepeat` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0324 | Engine | `EngineCommand::PlayPattern` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0325 | Engine | `EngineCommand::PlayFromPattern` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0326 | Engine | `EngineCommand::SetSoloPattern` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0327 | Engine | `EngineCommand::SetPreviewPattern` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0328 | Engine | `EngineCommand::Reset` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0329 | Engine | `EngineCommand::ClearAllModules` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0330 | Engine | `EngineCommand::SetMasterVolume` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0331 | Engine | `EngineCommand::SetGlideTime` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0332 | Engine | `EngineCommand::SetFocusedInstrument` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0333 | Engine | `EngineCommand::SetBypass` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0334 | Engine | `EngineCommand::AddVisualizer` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0335 | Engine | `EngineCommand::RemoveVisualizer` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0336 | Engine | `EngineCommand::AddEffectInstance` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0337 | Engine | `EngineCommand::RemoveEffect` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0338 | Engine | `EngineCommand::ReorderEffect` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0339 | Engine | `EngineCommand::SetEffectChainOrder` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0340 | Engine | `EngineCommand::SetEffectParameter` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0341 | Engine | `EngineCommand::SetEffectEnabled` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0342 | Engine | `EngineCommand::SetSong` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0343 | Engine | `EngineCommand::SetModGrid` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0344 | Engine | `EngineCommand::ArmRecord` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0345 | Engine | `EngineCommand::DisarmRecord` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0346 | Engine | `EngineCommand::SetMetronome` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0347 | Engine | `EngineCommand::SetMetronomeVolume` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0348 | Engine | `EngineCommand::SetAudioInputConsumer` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0349 | Engine | `EngineCommand::ClearAudioInputConsumer` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |
| CAP-0350 | Engine | `EngineCommand::LoadSampleData` | GUI, MCP bridge, render CLI — via the command ring (`LIMIT-0012`) | | | | Discovered |

### `EngineEvent` variants (14)

| ID | Surface | Capability | Reachable from | Disposition | V2 owner/replacement | Evidence | Status |
|----|---------|------------|----------------|-------------|----------------------|----------|--------|
| CAP-0351 | Engine | `EngineEvent::PeakMeter` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |
| CAP-0352 | Engine | `EngineEvent::RmsMeter` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |
| CAP-0353 | Engine | `EngineEvent::VoiceCount` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |
| CAP-0354 | Engine | `EngineEvent::ParameterChanged` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |
| CAP-0355 | Engine | `EngineEvent::CpuUsage` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |
| CAP-0356 | Engine | `EngineEvent::BufferUnderrun` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |
| CAP-0357 | Engine | `EngineEvent::EnvelopeStage` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |
| CAP-0358 | Engine | `EngineEvent::WaveformData` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |
| CAP-0359 | Engine | `EngineEvent::NoteTriggered` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |
| CAP-0360 | Engine | `EngineEvent::NoteReleased` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |
| CAP-0361 | Engine | `EngineEvent::AllNotesReleased` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |
| CAP-0362 | Engine | `EngineEvent::KeyRangeLearned` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |
| CAP-0363 | Engine | `EngineEvent::RecordingPreview` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |
| CAP-0364 | Engine | `EngineEvent::RecordedNotesFlushed` | Engine → GUI, MCP, OSC sender — via the prioritized event rings (`LIMIT-0013`) | | | | Discovered |

### Module types (75)

| ID | Surface | Capability | Reachable from | Disposition | V2 owner/replacement | Evidence | Status |
|----|---------|------------|----------------|-------------|----------------------|----------|--------|
| CAP-0365 | Modules | `ModuleType::Oscillator` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0366 | Modules | `ModuleType::MathOscillator` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0367 | Modules | `ModuleType::SubOscillator` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0368 | Modules | `ModuleType::Noise` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0369 | Modules | `ModuleType::Filter` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0370 | Modules | `ModuleType::Envelope` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0371 | Modules | `ModuleType::Lfo` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0372 | Modules | `ModuleType::Amplifier` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0373 | Modules | `ModuleType::Mixer` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0374 | Modules | `ModuleType::StereoOutput` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0375 | Modules | `ModuleType::Delay` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0376 | Modules | `ModuleType::Reverb` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0377 | Modules | `ModuleType::Distortion` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0378 | Modules | `ModuleType::Chorus` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0379 | Modules | `ModuleType::Phaser` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0380 | Modules | `ModuleType::Flanger` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0381 | Modules | `ModuleType::Compressor` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0382 | Modules | `ModuleType::Eq` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0383 | Modules | `ModuleType::Waveshaper` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0384 | Modules | `ModuleType::Oscilloscope` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0385 | Modules | `ModuleType::LevelMeter` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0386 | Modules | `ModuleType::SpectrumAnalyzer` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0387 | Modules | `ModuleType::ModMatrix` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0388 | Modules | `ModuleType::RingMod` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0389 | Modules | `ModuleType::EnvelopeFollower` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0390 | Modules | `ModuleType::WavetableOsc` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0391 | Modules | `ModuleType::Mseg` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0392 | Modules | `ModuleType::AdditiveOsc` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0393 | Modules | `ModuleType::BbdDelay` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0394 | Modules | `ModuleType::MidSide` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0395 | Modules | `ModuleType::Limiter` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0396 | Modules | `ModuleType::Euclidean` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0397 | Modules | `ModuleType::TuringMachine` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0398 | Modules | `ModuleType::RandomGates` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0399 | Modules | `ModuleType::KeyboardPanner` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0400 | Modules | `ModuleType::BodyResonance` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0401 | Modules | `ModuleType::MechanicalNoise` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0402 | Modules | `ModuleType::GranularOsc` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0403 | Modules | `ModuleType::Convolver` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0404 | Modules | `ModuleType::PhaseVocoder` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0405 | Modules | `ModuleType::KineticModulator` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0406 | Modules | `ModuleType::SignalMonitor` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0407 | Modules | `ModuleType::FrequencyShifter` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0408 | Modules | `ModuleType::VectorMixer` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0409 | Modules | `ModuleType::LaSynth` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0410 | Modules | `ModuleType::PitchTracker` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0411 | Modules | `ModuleType::EnsembleChorus` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0412 | Modules | `ModuleType::ShimmerReverb` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0413 | Modules | `ModuleType::GranularFx` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0414 | Modules | `ModuleType::SpectralBlur` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0415 | Modules | `ModuleType::ModalResonator` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0416 | Modules | `ModuleType::ReverseGateReverb` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0417 | Modules | `ModuleType::FractalOsc` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0418 | Modules | `ModuleType::Sampler` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0419 | Modules | `ModuleType::AudioInput` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0420 | Modules | `ModuleType::LadderFilter` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0421 | Modules | `ModuleType::DriftGenerator` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0422 | Modules | `ModuleType::ChaoticOsc` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0423 | Modules | `ModuleType::FormantFilter` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0424 | Modules | `ModuleType::Fooglers` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0425 | Modules | `ModuleType::BeatDetector` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0426 | Modules | `ModuleType::PadSynth` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0427 | Modules | `ModuleType::AmFormant` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0428 | Modules | `ModuleType::TiltEq` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0429 | Modules | `ModuleType::Univibe` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0430 | Modules | `ModuleType::CrossoverSplitter` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0431 | Modules | `ModuleType::Vocoder` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0432 | Modules | `ModuleType::TransientShaper` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0433 | Modules | `ModuleType::VoiceSynth` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0434 | Modules | `ModuleType::VocalTract` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0435 | Modules | `ModuleType::Fof` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0436 | Modules | `ModuleType::Script` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0437 | Modules | `ModuleType::AudioScript` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0438 | Modules | `ModuleType::SidOscillator` | Patch editor, MCP `add_module`, project load | | | | Discovered |
| CAP-0439 | Modules | `ModuleType::SpatialPanner` | Patch editor, MCP `add_module`, project load | | | | Discovered |

### Built-in patches (68)

| ID | Surface | Capability | Reachable from | Disposition | V2 owner/replacement | Evidence | Status |
|----|---------|------------|----------------|-------------|----------------------|----------|--------|
| CAP-0440 | Patches | `patch_acid_bass` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0441 | Patches | `patch_aggressive_bass` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0442 | Patches | `patch_ambient_keys` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0443 | Patches | `patch_analog_dream_machine` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0444 | Patches | `patch_auto_wah_bass` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0445 | Patches | `patch_brown_drone` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0446 | Patches | `patch_bytebeat_glitch` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0447 | Patches | `patch_chaos_drone` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0448 | Patches | `patch_choir` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0449 | Patches | `patch_deep_space_pad` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0450 | Patches | `patch_digital_chime` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0451 | Patches | `patch_drum_hihat` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0452 | Patches | `patch_drum_kick` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0453 | Patches | `patch_drum_snare` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0454 | Patches | `patch_ethereal_shimmer_pad` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0455 | Patches | `patch_euclidean_texture` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0456 | Patches | `patch_expressive_lead` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0457 | Patches | `patch_fluid_keys` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0458 | Patches | `patch_fluid_pad` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0459 | Patches | `patch_fm_bell` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0460 | Patches | `patch_fof_choir` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0461 | Patches | `patch_formant_voice` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0462 | Patches | `patch_fractal_cosmos` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0463 | Patches | `patch_glitch_pad` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0464 | Patches | `patch_grand_piano` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0465 | Patches | `patch_granular_cathedral` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0466 | Patches | `patch_granular_storm` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0467 | Patches | `patch_harmonic_lead` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0468 | Patches | `patch_hybrid_resonator` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0469 | Patches | `patch_karplus_guitar` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0470 | Patches | `patch_kinetic_pad` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0471 | Patches | `patch_kinetic_pluck` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0472 | Patches | `patch_la_synth_pluck` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0473 | Patches | `patch_metallic_bell` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0474 | Patches | `patch_moog_resonant_sweep` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0475 | Patches | `patch_mseg_crystal_lead` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0476 | Patches | `patch_noise_sweep` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0477 | Patches | `patch_pitch_following_drone` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0478 | Patches | `patch_pluck_synth` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0479 | Patches | `patch_punchy_stab` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0480 | Patches | `patch_pwm_epiano` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0481 | Patches | `patch_resonant_percussion` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0482 | Patches | `patch_ring_mod_drone` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0483 | Patches | `patch_satb_alto` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0484 | Patches | `patch_satb_bass` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0485 | Patches | `patch_satb_soprano` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0486 | Patches | `patch_satb_tenor` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0487 | Patches | `patch_screamer_lead` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0488 | Patches | `patch_shepard_riser` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0489 | Patches | `patch_solo_voice` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0490 | Patches | `patch_spacey_bass` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0491 | Patches | `patch_spectral_drone` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0492 | Patches | `patch_spectral_freeze_pad` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0493 | Patches | `patch_stereo_unison_pad` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0494 | Patches | `patch_string_ensemble` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0495 | Patches | `patch_sub_bass` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0496 | Patches | `patch_unison_pwm_strings` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0497 | Patches | `patch_unison_supersaw` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0498 | Patches | `patch_unison_sync_lead` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0499 | Patches | `patch_vector_pad` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0500 | Patches | `patch_velocity_pad` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0501 | Patches | `patch_vintage_electric_piano` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0502 | Patches | `patch_vintage_lead` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0503 | Patches | `patch_vocal_pad` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0504 | Patches | `patch_vocal_tract` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0505 | Patches | `patch_warm_evolving` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0506 | Patches | `patch_wave_folder_bass` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |
| CAP-0507 | Patches | `patch_waveshaper_lead` | GUI patch browser, MCP `list_example_patches` / `load_example_patch` / `apply_example_patch` | | | | Discovered |

### Audio-host diagnostics

| ID | Surface | Capability | Reachable from | Disposition | V2 owner/replacement | Evidence | Status |
|---|---|---|---|---|---|---|---|
| CAP-0508 | Audio host | CPAL asynchronous stream-error classification, including `RealtimeDenied`, `Xrun`, route/device lifecycle, permission, resource, and configuration failures | CPAL output/input error callback → atomic bitset → `AudioStream::take_async_error`; GUI and MCP polling surface categorized diagnostics with device metadata or its explicit lookup failure through tracing, and a coalesced output `Xrun` also reaches `AudioProcessor::on_error`; EVD-0016 separately retains typed counters | | Phase 9 device lifecycle and structured diagnostics | `crates/pertylizer/src/audio/backends/cpal_backend.rs`; `EVD-0016` | Investigating |

The V1 shipping path now records from CPAL's potentially real-time worker using
atomics only. A pending output `Xrun` reaches the output processor without
allocation when another data callback occurs. Independently, non-real-time GUI
and MCP polling surfaces every category through tracing, including device loss
when no later data callback can occur. Diagnostics retain source-device metadata
or its explicit lookup failure, and a replaced stream is labeled as retired;
shutdown drains retained input and
output diagnostics. Repeated occurrences of one category coalesce between
polls, while EVD-0016 retains exact counts. There is still no structured
UI/event consumer for the remaining
categories. The stable labels for all known CPAL 0.18.2 kinds
prevent its richer errors from collapsing into indistinguishable text; the
required non-exhaustive fallback is visibly `unknown`. The resolved-version
evidence gate makes a later CPAL update fail until the versioned method is
reviewed, but Phase 9 still owns durable lifecycle and UI diagnostics.

Next free identifier after this section: `CAP-0510`.

## Project actions 2026-09-09

This is one bounded slice of **P00B-T002**, inspected at `cf2edf5b`. It classifies six document/history
shortcuts and the startup recovery offer, not the whole capability inventory. `Migrate` retains the user-facing
capability under the existing [Application Core](../architecture/application-core.md) and
[Project Core](../architecture/project-core.md) ownership targets. It does not require copying V1's engine commands,
GUI mirrors, history representation, file encoding or failure behavior into V2. No ADR is accepted here.

### Method and acceptance boundary

Start with the six document/history entries in `AppShortcut::ALL`, follow the keyboard and menu dispatch to each handler,
then read its state mutation and success/failure/cancellation branches. For recovery, start at app construction
and follow the store selection, prompt and restore/discard handlers. Inspect the callees rather than relying on
comments or the existence of a type. The source links below are relative to this checkout; the recorded revision
pins the observations if later code changes.

A row is classifiable only when a shipped entry point reaches the named behavior, its proposed owner follows
the existing architecture, and the unresolved implementation checks have an owner. A missing dispatch, a callee
that cannot perform the claimed action, or an owner inconsistent with the roadmap falsifies that classification.
Those defects block acceptance; optional implementation detail does not. This is **source-inspection evidence**:
no GUI interaction, filesystem failure injection, engine execution or migration test was run for these observations.
Existing tests named below were read, not executed. The rows are `Classified`, never `Verified`.

### New and Open

`CAP-0048` and `CAP-0049` are reachable with the default `gui-egui` feature in
[Cargo.toml](../../../crates/pertylizer/Cargo.toml). In
[egui_backend.rs](../../../crates/pertylizer/src/gui/egui_backend.rs), `menu_file` and `handle_app_shortcuts`
call `request_new_project` and `request_open_project`. Both check `is_dirty`; a dirty document arms a
`PendingAction` rather than immediately replacing it. The shortcut gate and shifted-binding precedence live in
[shortcuts.rs](../../../crates/pertylizer/src/gui/shortcuts.rs). Home's Open button has its own dispatch to the
same OpenProject dialog mode; Recent Projects uses `load_recent_project` behind its own dirty check.

In [project_flow.rs](../../../crates/pertylizer/src/gui/egui_backend/project_flow.rs),
`show_unsaved_changes_dialog` distinguishes save success, failure and a deferred path request. Save success runs
the pending action; an awaiting-path save retains it; failure or Cancel drops it. Don't Save calls `mark_saved`
and proceeds. `reset_to_new_project` clears history, invokes the project reset, rebuilds UI state and clears the
project path. The file-dialog result in
[dialog_flow.rs](../../../crates/pertylizer/src/gui/egui_backend/dialog_flow.rs) calls `project::load_file` and
dispatches project JSON, a standalone patch or a bundle. Project loading clears the sample library before apply;
bundle loading supplies embedded samples before apply. A patch is loaded into the active instrument, not opened
as a new whole-project document. These are branches of the existing smart-open surface, not three new CAP entries.

**Failure boundary:** `apply_and_refresh_project` clears history before calling
[project_apply::apply_project](../../../crates/pertylizer/src/project_apply.rs). It logs an apply error and
still refreshes UI mirrors; its return type cannot report that error to the open handler, which can then mark the
project saved and report a successful load. Reset similarly logs a failure and continues. Thus read/decode errors
are surfaced, but the inspection does **not** establish atomic project replacement or preservation of the previous
document on apply failure. Phase 10B/10C owns operation failure and revision handling; 10D owns decoding/conversion.

### Save and Save As

`CAP-0050` calls `save_current_project_outcome` in `project_flow.rs`: a known path goes to a write; an untitled
project opens the SaveProject dialog and returns `AwaitingPath`. `CAP-0051` always opens that dialog. Its result
handler in `dialog_flow.rs` performs the write and clears a deferred pending action on cancellation or failure;
only a reported successful write executes the pending action. Direct-save failure returns `Failed`.

Both write paths use `create_project_from_app`, which calls `build_project_from_engine` and then overlays active
instrument selection and GUI metadata through `overlay_ui_metadata`. Module positions, groups and canvas size
therefore come from the frontend at save time; see the ownership rows `STATE-0027`, `STATE-0031` and `STATE-0032`
in [state-ownership.md](state-ownership.md). These links identify data owners, not a re-verification of every
historical behavior claim in that ledger.

The handlers use [project::normalize_project_path](../../../crates/pertylizer/src/project.rs) and select
`ProjectFile::save` or [bundle::save_bundle](../../../crates/pertylizer/src/bundle.rs) according to whether the
sample library is nonempty. Both writers call the atomic I/O helper. After reported success, the GUI updates the
current path, calls `mark_saved` and updates recent projects. `mark_saved` captures a clean baseline and calls
`retire_recovery_snapshot`, which attempts retirement under both the current and previously snapshotted paths.
This covers Save As association changes in the code, not a tested race guarantee.

**Failure boundary:** sample-library lock failures can select a plain-project write; no claim of sample-complete
save under those failures follows from writer success. The GUI captures its baseline after writing; this is not
V2's revision-pinned save receipt. Phase 10A owns canonical snapshots, 10C owns save/dirty/recovery coordination,
and 10D owns format conversion and asset integrity. Choosing a V2 format or Save As identity/fork rule is outside this slice.

### Undo and Redo

`CAP-0052` and `CAP-0053` reach `execute_undo` and `execute_redo` in
[undo_flow.rs](../../../crates/pertylizer/src/gui/egui_backend/undo_flow.rs). The Edit menu enables each button
from `can_undo`/`can_redo`; the shortcuts call the same handlers and an empty stack yields no action. Undo first
refreshes a top-of-stack effect-addition snapshot, then obtains the inverse from
[UndoManager::undo](../../../crates/pertylizer/src/undo.rs). Redo obtains the original action from
`UndoManager::redo`. Both call `apply_undo_action`, whose branches mutate song/UI/sample state or send engine
commands. This is the reachable GUI history mechanism, not evidence that every GUI or MCP mutation is undoable.

`UndoManager` moves an entry between stacks **before** the application handler executes it. The handler returns
no common success receipt. `is_dirty` in `egui_backend.rs` uses history position first, then the untracked-mutation
latch and revision comparison; it does not simply equate equal stack depths with equal documents. The existing
`undoing_every_edit_returns_to_the_saved_position` and
`a_new_edit_after_undoing_past_the_save_is_not_the_saved_position` tests exercise the manager's position model,
not end-to-end undo application or error rollback. Phase 10B/10C owns canonical operations, history and failure
semantics. ADR-0015 remains the open representation question; no particular inverse-command design is mandated.

### Startup recovery

`CAP-0055` starts at `SynthApp::new` calling `check_for_recoverable_work`. In
[autosave_flow.rs](../../../crates/pertylizer/src/gui/egui_backend/autosave_flow.rs), that calls
[RecoveryStore::find_recoverable](../../../crates/pertylizer/src/recovery.rs), which prunes and selects the newest
eligible snapshot. `supersedes_manual_save` compares snapshot time to the manual file's modification time;
untitled snapshots and snapshots whose manual file is missing or unreadable are also eligible. `show_dialogs`
draws the pending recovery offer before other dialogs. Opening the store can fail and disable autosave with a log.

The per-frame `tick_autosave` checks dirty state, the pending recovery offer, the attempt interval, the last
snapshotted revision and whether a write is already in flight. Capture uses `create_project_from_app`; a worker
writes the snapshot and optional sample library. `poll_autosave` consumes its result without marking the manual
project saved. `accept_recovery` chooses bundle or plain loading, restores the original project path and marks
the result dirty after reported success. `decline_recovery` attempts to delete the snapshot. The existing
`a_snapshot_round_trips` and `a_snapshot_of_a_project_with_samples_keeps_the_samples` tests exercise store payloads;
they do not exercise the GUI prompt, all metadata, in-flight retirement or failed project application.

**Failure boundary:** recovery inherits the apply-error propagation limit above; the prompt is cleared after
Recover even when loading reports an error. Snapshot retirement does not wait for `in_flight`, so this inspection
does not establish that a worker cannot recreate a just-retired snapshot. These are verification targets, not
runtime reproductions or fixes in this slice. Phase 10C owns recovery coordination; 10D owns its document/assets.
`STATE-0054`/`STATE-0057` identify the runtime-session state. ADR-0024's recording-take commit semantics are a
different capability and do not determine whether this project-recovery offer migrates.

### Migration checks still owed

The [roadmap](../ROADMAP.md) already assigns these implementation boundaries. The checks below state what would
falsify successful migration of this subset; none has passed here and none closes P00B-T004/T005 or Phase 0B.

| Capabilities | Owner | Observable check before `Verified` |
|---|---|---|
| CAP-0048, CAP-0049 | 10B/10C lifecycle; 10D conversion; 11 GUI | Invoke through direct Application operations and the GUI adapter; dirty confirmation, deferred save, cancel and save failure must not accidentally replace the document. Inject open/apply failure and verify the reported effect and resulting revision agree with the state actually retained. Exercise project, patch and bundle branches under their explicit conversion dispositions. |
| CAP-0050, CAP-0051 | 10C save/history; 10D format/assets; 11 GUI | Save and reopen the captured canonical revision with editor metadata and samples intact. A later edit stays dirty; a failed write reports failure and does not advance the saved revision. Save As updates its destination/recovery association only on success. Exercise an in-flight autosave across save, Save As and discard. |
| CAP-0052, CAP-0053 | 10B operations; 10C history; 11 GUI | Undo/redo recorded edits through the canonical operation boundary; compare snapshots and stable references. Divergent edits at equal history depth must remain distinct. Inject operation failure and verify history, effect and revision stay consistent under the selected failure contract. |
| CAP-0055 | 10C recovery; 10D assets; 11 GUI | Recover a sample-bearing snapshot with metadata, assets and original save association intact and still dirty; test discard, unreadable/corrupt input and failed apply. Assert a pending offer is not overwritten and an in-flight worker cannot resurrect work after its retirement under the selected recovery contract. |

At the end of this project-action pass, every other capability disposition and the per-action GUI census remained
open. MCP/CLI/OSC behavior, live transport (`CAP-0054`), Phase 8 mixing/effects and public-facade decisions were
outside that pass; the later CLI inspection below covers only its declared subset.
The source traces above are supporting inventory evidence, not the complete workflow or round-trip evidence tasks.

## CLI entry points 2026-09-11

This bounded P00B-T002 slice inspects `CAP-0040`–`CAP-0043` and adds `CAP-0509` at source revision
`85d92f1b`. It follows the source-inspection method and classification threshold above: enumerate the runtime
binary's parser, follow each dispatch and its callees, and record failure limits and migration owners. The falsifier
is a missing parser branch, unreachable claimed behavior, or an owner inconsistent with the existing roadmap.
Such a defect blocks classification; optional implementation detail does not. No command execution, hardware test,
MCP request or migration test supports this pass. Existing tests cited below were read, not run.

`Migrate` retains these user-facing capabilities under the [roadmap](../ROADMAP.md), not V1's internal types or
failure behavior. It neither accepts an ADR nor authorizes a break to existing CLI or JSON contracts. In particular,
[ADR-0028](../decisions/ADR-0028-long-running-job-contract.md) still blocks a new V2 render/analysis surface before
its Phase 10B job contract; classifying an existing V1 command does not open that gate. This pass adds no code,
changes no evidence harness, and does not close P00B-T002 or Phase 0B.

### Parser coverage and mode selection

[main.rs](../../../crates/pertylizer/src/main.rs) defines one `Cli`: optional `--headless` (`mcp` feature),
optional `--no-osc` (`osc` feature), and the ungated `Render` and `Compare` subcommands. Their structs declare
**ten** render options and **four** compare options; generated help is excluded from those counts. Render has
`--protocol-version`, `--input`, `--output`, `--sample-rate`, `--bit-depth`, `--seconds`, `--tail-seconds`,
`--result-json`, repeatable `--solo-track` and repeatable `--mute-track`. Compare has `--protocol-version`,
`--reference`, `--candidate` and `--result-json`. This corrects CAP-0042's earlier eleven-argument count and
covers the previously omitted comparison command without renumbering any existing entry.

Parsing follows tracing and panic-hook installation but precedes the banner, Rayon pool and mode dispatch.
Unknown arguments/subcommands are rejected by Clap. With `mcp`, combining `--headless` and a subcommand exits
nonzero; otherwise a subcommand dispatches first, then headless mode, then GUI startup or a no-GUI error.
`--no-osc` is a top-level argument, not declared with Clap's `global = true`; only GUI startup reads its value.
Help/version are generated parser responses, not extra operational modes. The parser tests `no_arguments_parses`,
`an_unknown_argument_is_rejected`, `headless_is_unknown_without_the_mcp_feature` and
`render_parses_its_version_1_arguments` cover parsing, not successful startup or process-level stream behavior.
Feature gates come from [Cargo.toml](../../../crates/pertylizer/Cargo.toml); this is no claim that every possible
feature combination builds. Other developer binaries, examples and external consumers remain outside this census.

### GUI launch and OSC switch

`CAP-0040` reaches `run_gui`, which loads app settings and, when MCP or OSC is enabled, runtime configuration.
It constructs the V1 engine/session/song/sample library, starts the feature-enabled MCP HTTP thread, chooses an
audio host and invokes `create_backend().run`. Failure to create the default audio host selects the null host;
this fallback is not a guarantee that a later stream/window startup succeeds. Backend errors propagate.
MCP thread/runtime/server startup failures are logged and do not abort GUI startup. No GUI launch was performed.

`CAP-0043` acts earlier in the same function: with `osc` enabled, `--no-osc` skips construction of the OSC
telemetry object and shared state. Otherwise startup uses the runtime multicast group, port and update rate,
and calls `start` if a note-event consumer is available. The flag neither disables MCP nor changes saved project
content. Headless and both subcommands return before this branch, so they start no OSC telemetry here even when
the switch is absent. This classifies the suppression capability, not the address catalog (`CAP-0044`) or a
particular future configuration encoding. Phase 10E owns host/service policy and Phase 11 the GUI adapter.

### Headless stdio

`CAP-0041` reaches `run_headless_mcp` only with `mcp`. Despite its older function comment saying audio still
plays, its body selects `audio::null_host`, starts that output driver, constructs the V1 session and bridge, and
runs [serve_stdio](../../../crates/synth_mcp/src/lib.rs) in a Tokio runtime. That callee uses stdin/stdout,
waits for the service to finish, and propagates startup/session errors. This path does not start the GUI's HTTP
server or physical audio output. Tracing and startup messages target stderr; no complete tool-output audit is
claimed. The bridge's individual operations remain the MCP per-item rows, not classified by this entry.

After successful service completion, the function explicitly stops the host and drains asynchronous diagnostics.
Earlier `?` returns skip that explicit sequence; drop behavior and exceptional shutdown remain unverified here.
Phase 10E owns transport and disconnect/error conformance over Application Core/runtime-session services. This
retains stdio automation, not an assertion that headless means device playback or that every request is atomic.

### Render command

`CAP-0042` reaches `render_project`, which rejects a mismatched protocol version before constructing a
[RenderCommand](../../../crates/pertylizer/src/render/command.rs). The command validates duration/rate bounds,
checks input/output path collisions, obtains the input digest, loads through
[render/headless.rs](../../../crates/pertylizer/src/render/headless.rs) and
[project_apply.rs](../../../crates/pertylizer/src/project_apply.rs), resolves the requested mix, renders and writes
a WAV, and builds the version-1 [receipt](../../../crates/pertylizer/src/render/receipt.rs).
The loader drives its own engine while its file-loading thread applies a project, bundle or standalone patch;
it does not open a device. A standalone patch creates no arrangement here and can produce a no-instrument-signal
warning. [mix.rs](../../../crates/pertylizer/src/render/mix.rs) resolves track IDs or unique names and rejects
unknown/ambiguous selections and solo/mute overlap before clearing and replacing saved track solo/mute flags.
The render scope includes master and return effects; these in-memory mix overrides are not saved back.

Warnings, including load diagnostics, reach the receipt and stderr. Without `--result-json`, the receipt goes
to stdout; with it, the command writes the receipt file. Run errors are printed and exit nonzero. The WAV write
precedes output digesting and receipt writing, so a later failure can leave a completed WAV without a receipt;
this is not an atomic pair. Input hashing and loading reopen the path separately, so this source inspection
cannot establish that the digest names the loaded bytes under concurrent replacement. Path guards use
`path_identity`; they are not a general filesystem-race guarantee. These limits belong to Phase 10B's captured
job input/result lifecycle and Phase 10D's I/O, rather than becoming V2 guarantees by classification.

**Conflicting prior evidence:** ADR-0028's workflow table says the CLI input cannot change because it loads a
file. The separate digest/load reads above refute that claim under concurrent replacement; the nearby comment
in `render/command.rs` overstates the guarantee for the same reason. ADR-0028 remains `Deferred`, and its
Phase 10B owner must correct that pinning assertion before accepting the job contract or relying on it for a
new consumer. This inventory records the contradiction rather than inheriting the assertion; it does not change
the ADR's deferral or assert that V1 already captures immutable input bytes.

The existing `the_wav_parses_and_matches_the_receipt` and `an_output_that_is_the_input_is_refused` tests in
[render_command.rs](../../../crates/pertylizer/tests/render_command.rs) exercise the library command's receipt
and collision branches. They do not execute the CLI or verify all filesystem interleavings. This pass makes no
V1/V2 parity claim and adds no engine selector, progress, cancellation or shared render request.

### Compare command

`CAP-0509` reaches `compare_renders`, rejects a mismatched comparison protocol version and calls
[run_compare_command](../../../crates/pertylizer/src/compare/command.rs). It reads two already rendered WAVs,
checks path collisions, computes digests, decodes through
[Signal::load](../../../crates/pertylizer/src/compare/signal.rs), measures differences and emits the version-1
[ComparisonReport](../../../crates/pertylizer/src/compare/report.rs). The decoder rejects unsupported sample
formats, empty inputs and inputs exceeding its decoded-size bound. Same resolved input paths and a report path
resolving to an input are refused; distinct files with equal bytes are allowed. As with render, hashing and
loading are separate reads and do not pin bytes against concurrent replacement.

The command reports sample, level, timing, pitch, envelope, stereo, spectrum and loudness differences.
Inapplicable measurements are optional and accompanied by warnings; a difference is not an execution failure
or a parity verdict. JSON goes to stdout or `--result-json`, with warnings on stderr and command errors exiting
nonzero. This utility performs no project loading or rendering and is the retained synchronous Phase 0A tool
recognized by ADR-0028, not multi-project render orchestration.

[two_different_projects_report_a_difference and the other comparison tests](../../../crates/pertylizer/tests/compare_command.rs)
exercise the library command's changed-input, same-path refusal and parseable-report behavior. They are evidence
of existing test intent, not a new measurement or a process-level CLI test run in this pass. Phase 10E owns
retaining this analysis/CLI surface; any later job integration must obey Phase 10B's contract.

### CLI migration checks still owed

| Capabilities | Owner | Observable check before `Verified` |
|---|---|---|
| CAP-0040 | 9 host; 10E configuration/services; 11 GUI | Launch the GUI through its shipped entry point; exercise audio and service startup failures and ensure the visible runtime state agrees with the actual outcome. Test the no-GUI build's refusal separately from parser acceptance. |
| CAP-0041 | 10E MCP/service adapter; runtime-session lifecycle | Start stdio without a GUI or physical audio device; send representative discovery and mutation requests through the common operation boundary. Check clean protocol stdout, startup failure, EOF/disconnect and error-path teardown. Feature-disabled builds must reject the flag. |
| CAP-0042 | 10A snapshot; 10B jobs; 10D I/O/assets; 10E CLI | Render converted project/bundle/patch inputs under their fidelity dispositions and a captured revision; check mix selection, WAV settings, load diagnostics and receipt agreement. Replace input bytes during capture and fail WAV/receipt writes; the result must identify what was actually consumed and retained. Check version refusal and stdout/stderr/exit behavior in a subprocess. |
| CAP-0043 | 10E host configuration/telemetry; 11 GUI | With OSC compiled in, compare GUI startup with and without the switch: suppression must start no telemetry sender and leave project state unchanged. Exercise feature-disabled argument rejection and mode combinations; document any approved change to the accepted invocation syntax. |
| CAP-0509 | 10E analysis/CLI adapter; 10B only for later job integration | Compare distinct equal-byte files and deliberately different signals; check versioned reports, unavailable measurements and their diagnostics, path refusals, and nonzero exit on execution failure. A numerical difference must not become an automatic acceptance verdict. Pin or diagnose input replacement under the selected I/O contract. |

The CLI subset is classified, not migration-verified. The subsequent MCP project-operation pass below addresses
four of the remaining MCP dispositions. Other MCP operations, OSC messages, engine/module/catalog entries,
public APIs, the build matrix, developer tools and the per-action GUI census remain open. CAP-0044–CAP-0047
and live transport CAP-0054 keep their existing status.

## MCP project operations 2026-09-11

This bounded P00B-T002 slice classifies four existing tools at `bab84945`: `load_project` (CAP-0172),
`new_project` (CAP-0173), `save_patch` (CAP-0205) and `save_project` (CAP-0206). It uses the existing
[source-inspection method](#method-and-acceptance-boundary): follow registered dispatch through the bridge to
mutation or persistence, inspect failure branches, and assign owners under the
[Application Core target](../architecture/application-core.md) and [roadmap](../ROADMAP.md).
A missing dispatch, a false behavioral claim or an inconsistent owner blocks classification; optional detail does
not. No MCP call, GUI interaction, runtime experiment or migration test was executed for this inspection.
The tests cited below were read, not run. No evidence harness, production code or external contract changes.

`Migrate` retains the capability, not the misleading catalog wording, V1's mirrors or its partial-failure behavior.
It does not accept proposed ADRs, choose a V2 format or authorize a compatibility break. Core lifecycle, save and
I/O behavior belongs to 10A–10D; 10E adapts it to MCP and 11 consumes its GUI notifications. `Classified` does
not mean V2 has implemented it. The two preceding inspections cover GUI project actions and CLI entry points;
this pass does not classify all project-related MCP tools or close P00B-T002, P00B-T004/T005 or Phase 0B.

### Shared MCP entry and result boundary

[project.rs](../../../crates/synth_mcp/src/server/tools/project.rs) defines the four handlers, each annotated
`destructive_hint = true` and publishing `action_output_schema`. The default router in
[server.rs](../../../crates/synth_mcp/src/server.rs) includes `project_tool_router`; its batch dispatch also names
all four handlers. The GUI HTTP and headless stdio launch paths are covered by CAP-0040/0041. This is source
reachability in the default MCP-enabled application, not a request sent to a running server.

`new_project` takes `NoParams`; load/save take `ProjectPathParam.path`; patch export additionally requires an
`InstrumentId`. File handlers reject relative paths and `..` components before entering the bridge. This check
is lexical path validation, not filesystem authorization or protection against every symlink/race. The three file
handlers use `block_in_place`; `new_project` calls its synchronous bridge directly. `action_ok`, `action_failed`
and `action_rejected` return a message plus one structured mutation item. `stamp_outcome` sets the outcome
metadata and `isError`; a bridge error is a failed item, but says nothing about rollback of earlier mutations.
An `Ok` load summary containing reconstruction diagnostics still goes through `action_ok`, not a partial-item
result. This pass does not infer exact effects or atomicity from the one-item wrapper.

[ProjectBridge's implementation](../../../crates/pertylizer/src/mcp_bridge/project.rs) forwards to `do_*`
helpers in [mcp_bridge.rs](../../../crates/pertylizer/src/mcp_bridge.rs). Those helpers hold
`project_io_lock`; ordinary edits and sample imports need not take it, so it does not freeze the document or
isolate a save from every other mutation. `record_io_result` stores the last outcome and increments
`project_revision` for both successes and errors that reach it, including saves. Earlier path/instrument
validation failures do not pass through that helper. This counter is a GUI notification signal, not the
canonical revision required by Phase 10A.

On successful load/reset the bridge publishes a one-slot `ProjectRefresh` and updates source-path/author state.
[drain_mcp_state](../../../crates/pertylizer/src/gui/egui_backend/engine_events.rs) consumes that payload,
refreshes GUI mirrors and calls `mark_saved`. Save-only completion publishes status without a refresh payload,
so it does not itself change the GUI's current project path or mark the document saved. The last-result and
refresh slots are not a per-request receipt history. These are current adapter consequences; Phase 10B/10C owns
the operation, revision and saved-state relationship before Phase 11 adopts it.

### MCP project reset

`CAP-0173` reaches `do_new_project` and
[reset_to_new_project](../../../crates/pertylizer/src/project_apply.rs). The latter creates an empty `Untitled`
project with default globals, clears the sample library and passes it to `apply_project`. That stops transport,
tears down instruments and replaces song/mix state. There is no dirty-document confirmation in this MCP handler.
On reported success the bridge clears shared author and loaded path, publishes `ProjectRefresh::Reset`, and
clears the mix-comparison baseline; failure is returned and recorded without publishing that success refresh.

**Failure limit:** sample clearing and engine/song changes happen before all fallible apply steps have completed.
A returned error does not restore the previous project or samples. This is a lifecycle operation to migrate
through Application Core, not evidence that resetting V1 is transactional or that a failed call changed nothing.
`new_project_works_without_gui` in
[mcp_project_load.rs](../../../crates/pertylizer/tests/mcp_project_load.rs) checks the bridge's reset payload,
notification increment, cleared path and default tempo; it is not an all-failures rollback test.

### MCP project load

`CAP-0172` reaches `do_load_project` → `load_project_inner`. It calls
[project::load_file](../../../crates/pertylizer/src/project.rs), which detects a ZIP or decodes JSON. A project
clears the existing sample library before apply; a bundle is loaded into that library by
[load_bundle](../../../crates/pertylizer/src/bundle.rs). Successful apply updates shared author immediately,
stashes the decoded project for GUI refresh, records the loaded path and clears the mix-comparison baseline.
The returned string includes the apply report's diagnostic summary. No handler-side dirty confirmation occurs.

**Catalog discrepancy:** the `load_project` tool description advertises single-patch loading, and `save_patch`'s
description says its output is read back by `load_project`. The actual `LoadedFile::Patch` branch refuses with
"File is a single-instrument patch — use load_patch instead of load_project". Unlike GUI smart-open and the
CLI renderer's loader, this MCP operation accepts projects/bundles and rejects standalone patches. CAP-0172 is
classified for that implemented scope. The two tool descriptions need to match it; enabling patch acceptance
would be a separate behavior decision, not a documentation correction made by this pass.
The refusal is also misleading: the MCP router/dispatch has no `load_patch` tool. Its example-patch tools do
not accept arbitrary saved patch paths. There is no dedicated MCP file-import route for reopening `save_patch`
output at this revision; GUI smart-open and the CLI renderer provide separate patch-consuming surfaces.
Phase 10E must make the catalog and refusal agree with the supported routes. Any new MCP patch-file loader
needs its own capability entry or an explicitly approved expansion of CAP-0172.

**Failure limit:** a bundle loader clears the library before opening/parsing the archive and populates it
incrementally. `apply_project` tears down existing instruments before installing replacements, mutates the song,
and can fail later, including when the command-drop counter increased. Parse/apply errors therefore cannot all
be treated as preservation of the previous project. Nonfatal reconstruction diagnostics remain in a successful
summary, while some bundle decode losses never reach that summary: failed WAV decoding skips the sample and
prints only to stderr; malformed `metadata.json` is silently discarded via `.ok()`, losing its stored sample
names, root notes, loops and crops while WAV loading continues. The bridge does not provide a complete typed
effect/diagnostic receipt or a canonical revision. Phase 10B owns failure/state agreement; 10D
owns decode, conversion and asset validation. `load_project_works_without_gui` checks instrument count, tempo,
path and refresh/status publication, not every decoded field, partial apply or failed-bundle outcome.

### MCP project save

`CAP-0206` reaches `do_save_project` → `build_project_for_persistence`. It first waits for pending engine
commands and asks [McpSharedState::request_gui_project](../../../crates/pertylizer/src/mcp_shared.rs) for the
project the GUI would save. The GUI's
[service_mcp_project_requests](../../../crates/pertylizer/src/gui/egui_backend/project_flow.rs) re-drains MCP
refresh state and reconciles editors before `create_project_from_app` supplies its metadata overlay. With no
GUI or no reply before the timeout, the bridge builds from engine/song snapshots and shared save options.
An attached-GUI timeout logs a warning; it is not made a failed save or added to a structured loss report.
The state ledger's `STATE-0027`, `STATE-0031` and `STATE-0032` identify metadata whose GUI overlay can be lost.

After building, it takes one sample-library read guard for both bundle selection and writing. With samples,
`normalize_project_path` chooses `.ptz.zip`; without samples it preserves `.ptz`/`.json` and normalizes other
extensions to `.ptz`. The result string reports the actual written path. This shared library guard makes that
branch decision consistent with the writer's library, but the project was captured earlier: it does not establish
one atomic project-plus-assets revision. The plain writer calls `ProjectFile::save`, the bundle writer calls
`save_bundle`; both reach [atomic I/O](../../../crates/pertylizer/src/io/atomic.rs), writing a temporary file
before replacing the destination. Destination replacement is separate from correctness of the captured content.

**Failure limit:** the pre-drain and engine builder discard the boolean from `wait_for_pending_commands`.
That function can time out; its enqueue frontier also excludes commands already dropped. Current control
snapshots can already reflect accepted commands, so timeout alone does not prove a stale graph, but successful
save does not certify completed DSP application or a coherent snapshot under concurrent edits. GUI fallback can
lose metadata and separate snapshot reads are not a revision pin. Phase 10A/10C/10D owns canonical capture,
save/dirty coordination and assets. This does not reopen ADR-0028's gate for a new V2 render-job surface.

In [mcp_gui_project_snapshot.rs](../../../crates/pertylizer/tests/mcp_gui_project_snapshot.rs),
`an_mcp_save_persists_the_gui_built_project` uses a fake GUI responder and checks its marker song name;
`a_headless_mcp_save_falls_back_to_the_engine_build` checks the fallback's name. Neither proves the full live
GUI overlay. `save_project_works_without_gui` checks a reparseable file containing instruments and an I/O success
status. These are existing bounded tests, not verification of every persisted field or a new test run here.

### MCP patch export

`CAP-0205` validates the instrument through `ProjectBridge`, then `do_save_patch` calls
[save_patch_to](../../../crates/pertylizer/src/project_apply.rs). It waits for commands and instrument visibility,
finds the selected snapshot, filters its modules/connections, builds a `Patch`, creates parent directories and
calls `Patch::save` through atomic I/O. At the MCP boundary, an initially unknown instrument is rejected by
`validate_instrument` before this helper or I/O-result recording. The helper separately diagnoses a known
instrument not mirrored before the wait expires; its own not-found branch can also handle removal between the
bridge's validation and snapshot lookup. The path is used as supplied after the tool's path validation; this helper does not
apply whole-project extension normalization or write a sample bundle.

`build_patch_from_engine` starts from `Patch::new`, carries patch description/color, module descriptions,
parameters/scripts, connections and effect-chain order, and writes default module positions. It does not ask the
GUI for groups/layout/visualizer overlays or embed the sample library. This is an engine-derived instrument
export, not a complete project save or evidence that every GUI-authored patch field survives. As with project
save, the command-wait result is not propagated. The instrument lookup can fail, but a successful lookup does
not certify all prior DSP mutations or pin the graph against a concurrent edit.

`save_patch_writes_a_loadable_single_instrument_patch` in `project_apply.rs` checks that general `load_file`
recognizes the result as a patch and excludes a second instrument's distinctive module. It does not call MCP
`load_project`, so it does not contradict that tool's patch refusal. `save_patch_reports_an_unknown_instrument`
in [mcp_build_and_save_robustness.rs](../../../crates/pertylizer/tests/mcp_build_and_save_robustness.rs) checks
the bridge's unknown-ID error. Phase 10A/10D must define retained instrument metadata and asset references;
10B/10E owns the export operation and its MCP result.

### MCP project migration checks still owed

| Capabilities | Owner | Observable check before `Verified` |
|---|---|---|
| All four | 10B operation results; 10E MCP adapter | Exercise direct and batched calls over the supported transports. Assert required-input/path refusals, declared output schemas, `isError`, diagnostics and resulting state agree. A failed item must not imply rollback that did not occur; written files must not be claimed undone by batch restoration. Catalog descriptions must match implemented file-kind support, and refusals must not recommend nonexistent tools. |
| CAP-0173 | 10A document; 10B lifecycle; 10C history/saved state; 11 GUI | Reset a nonempty project with samples and mirrors. Verify the canonical result, source association and history under the chosen lifecycle rule; inject failure after mutation begins and assert the receipt describes the state retained. |
| CAP-0172 | 10B lifecycle; 10C saved state; 10D I/O/assets; 11 GUI | Load project/bundle cases and exercise standalone-patch refusal or its separately approved replacement contract. Test corrupt archives, malformed sample metadata, undecodable WAV entries, asset and reconstruction failures with nonempty prior state. Verify operation diagnostics, final document/assets and GUI refresh agree, including partial failure and nonfatal loss; missing samples or discarded metadata must not be silent or stderr-only. |
| CAP-0206 | 10A snapshot; 10B operation results; 10C save; 10D writer/assets; 10E MCP; 11 GUI integration | Compare equivalent GUI, MCP and headless saves of a canonical revision, including retained metadata/assets. Exercise a stalled/detached GUI, pending/dropped commands, concurrent edits/imports and failed writes. The result must identify the captured revision and actual path; a later edit stays dirty and failure cannot silently advance the saved revision. |
| CAP-0205 | 10A instrument content; 10B export; 10D format/assets; 10E MCP | Export one of two distinguishable instruments and reopen through the declared patch-file consumer (currently GUI smart-open/CLI render, with no dedicated MCP file-import route). A future MCP loader requires separate scope and classification. Verify the selected instrument's retained metadata and sample-reference policy, unknown/stale IDs, concurrent edits, wait expiry and write failure. No other instrument or song may leak into a standalone patch. |

This pass gave only these four MCP rows a disposition. The subsequent pass below addresses cleanup, lint and
example-patch tools. Other MCP tools and the remaining GUI/catalog/service inventories stay open; arbitrary
patch-file loading through MCP is still the missing route identified above. The project-tool
rollup's member names are corrected to include `optimize_project`; `lint_project` is declared in discovery.
No new CAP identifier is allocated, and the next free identifier remains CAP-0510.

## MCP cleanup, lint and example patches 2026-09-12

This P00B-T002 slice follows five registered tools at `ee5c6600` using the existing
[source-inspection method](#method-and-acceptance-boundary). A missing shipped dispatch, false behavioral claim,
inconsistent V2 owner or untestable migration criterion blocks classification; optional implementation detail
does not. The [Application Core target](../architecture/application-core.md) and [roadmap](../ROADMAP.md)
assign the owners. Source and named tests were read; no MCP calls, GUI interactions, runtime experiments or
Rust tests were executed. This pass changes no production code, evidence harness, external contract or ADR status.
`Migrate` retains each capability; it does not promise to reproduce V1's incomplete results or GUI side effects.

### Shared entry and result limits

The default routers and batch dispatch in [server.rs](../../../crates/synth_mcp/src/server.rs) reach all five
handlers. CAP-0040/0041 cover GUI HTTP and headless stdio startup. `lint_project`, `list_example_patches` and
`optimize_project` take `NoParams` and return typed JSON reports/listings. The first two carry read-only
annotations; optimization and both patch mutations carry destructive annotations. A produced lint report may
contain error diagnostics without being an MCP execution failure. Likewise, an optimization report carries
removal names/counts, not a partial-effect verdict or stable removed IDs.

`load_example_patch` takes a required name and returns a one-item action result. `apply_example_patch` takes a
required patch name and optional instrument ID, returning `ApplyExamplePatchResult`. Its handler explicitly
marks nonempty `errors` as `Partial` in outcome metadata; `isError` stays false for partial results and becomes
true for failures. Batch dispatch preserves that stated outcome. The load handler instead uses `action_ok` for
any bridge `Ok`. These wrappers do not establish atomic mutation, undo coverage or completed DSP application.
The [preceding pass](#shared-mcp-entry-and-result-boundary) describes the common action-result boundary.

### MCP project cleanup

`CAP-0178` runs [ProjectBridge::optimize_project](../../../crates/pertylizer/src/mcp_bridge/project.rs).
[Song::remove_unused](../../../crates/synth_sequencer/src/song.rs) treats arrangement placements as the roots:
it removes unplaced patterns and tracks, bumps the structure revision even for a no-op, and retains instruments
referenced by the remaining tracks. An unplaced pattern or an instrument kept only for manual playing is not a
root of this cleanup. The bridge then attempts removal of every unreferenced instrument in its snapshot and
calls [prune_unused_samples](../../../crates/pertylizer/src/project_apply.rs). Sample roots are the live
instrument `Sampler` modules' selected sample IDs, not a scan of every persisted reference or future asset type.
Removed sample names are sorted by sample ID; the result sums the four removal-list lengths.

**Failure/capture limit:** the bridge discards failed `remove_instrument` results with `.is_ok()` and still
returns `Ok(OptimizeResult)`. [SynthSession::remove_instrument](../../../crates/pertylizer/src/session.rs)
clears registry/counter/metadata state before a fallible command send, so omission from the removed list does
not prove no change. Song mutation, instrument snapshots and the later sample-root scan are separate steps,
without the project-I/O lock, a canonical revision pin or rollback. A report does not certify one coherent
pruning decision under concurrent edits or pending commands. Phase 10A/10B/10D must make the root policy and
resulting document/assets explicit; 10C owns history and dirty state. This pass retains cleanup, not an approval
to discard content outside a separately declared V2 reachability policy.

`prune_unused_samples_keeps_referenced_drops_orphans` in `project_apply.rs` pumps the engine, then checks one
referenced sample survives and one orphan is removed. It does not exercise the whole MCP optimizer, failed
instrument deletion, concurrent imports or every reference class.

### MCP project lint

`CAP-0153` uses the default [InstrumentBridge::lint_project](../../../crates/synth_mcp/src/bridge.rs): list
instruments, collect graph diagnostics, then append orphaned-track and hidden-event reports supplied by the
[application bridge](../../../crates/pertylizer/src/mcp_bridge/instruments.rs). Instruments disappearing between
listing and diagnostics are skipped; other bridge errors abort the report. `instruments_checked` counts the
collected diagnostic sets. `build_lint_report` counts every severity but includes only instruments with warnings
or errors in `entries`, retaining their informational context. Orphaned tracks add errors; each hidden-event
pattern adds one warning. The application checks note onsets and automation at/after a positive pattern length;
zero-length patterns are skipped. These are separate reads, not validation of a pinned complete project revision.

**Catalog/treatment limit:** the tool advertises feedback-loop and missing-audio-path checks. The application
implementation checks presence of source/output/envelope types and module-level cable participation, with
exceptions for effects and parameter-routed modulators. It does not traverse paths to prove source-to-output
reachability or detect cycles. It neither measures sound nor validates all assets, master/return graphs or the
persisted schema. A zero-error/zero-warning report therefore is not a proof that the project renders correctly.
The catalog must describe the implemented coverage; adding deeper analysis is separate work. Phase 10A owns
validation/diagnostic coverage and 10E its read adapter, not a new independent mutation authority.

`lint_project_surfaces_orphaned_track_instrument_references` in
[mcp_project_load.rs](../../../crates/pertylizer/tests/mcp_project_load.rs) checks one missing instrument reference
and the error count. `lint_report_tests` in `bridge.rs` check aggregation, actionable entries and empty input.
Neither establishes cycle detection or whole-project validation.

### MCP example-patch discovery

`CAP-0156` reads [categorized_patches](../../../crates/pertylizer/src/patches/mod.rs) through
[InstrumentBridge::list_example_patches](../../../crates/pertylizer/src/mcp_bridge/instruments.rs). It constructs
the built-in catalog and emits each patch's name, category, description, tags and declared module/connection
counts in catalog order. Counts describe the definition, not what a particular application path can reconstruct.
There is no directory scan or arbitrary-file import. Both mutation tools select the first catalog name matching
ASCII case-insensitively; they do not use a stable catalog ID or reject ambiguous duplicate names. Unknown names
return `PatchNotFound`. Phase 10A owns catalog definitions and their identity/content rules; 10E owns discovery
and selectors. Classifying these tools does not classify or fidelity-verify every built-in patch row.

### MCP example-patch load and GUI consumer

`CAP-0171` finds a built-in patch, adds a new instrument named after it, and immediately calls
[SynthSession::apply_patch](../../../crates/pertylizer/src/session.rs). It is not delayed until a GUI frame, despite
`apply_example_patch`'s contrasting catalog wording. Instrument creation failure is returned, but the load bridge
discards the apply report, then attempts to replace `pending_patch` and bumps the GUI notification revision.
The pending value holds only `(Patch, name)`, without the newly created instrument ID; a poisoned mutex silently
skips publication. The success message names the new instrument even when application reported diagnostics.

**GUI consequence:** [drain_mcp_state](../../../crates/pertylizer/src/gui/egui_backend/engine_events.rs) takes the
one-slot payload and calls [load_patch_data](../../../crates/pertylizer/src/gui/egui_backend/project_flow.rs).
That marks dirty and loads into the GUI's then-active instrument (or creates one if none is active).
[patch_bridge::load_patch](../../../crates/pertylizer/src/gui/patch_bridge.rs) clears and reconstructs that
instrument's graph as well as its editor metadata. This is another mutation, not just a label/cache refresh, and
can target an existing instrument other than the one named in the MCP reply. Multiple pending loads can overwrite
the one-slot payload. No request-specific completion or GUI-side diagnostic is returned to the original caller.
10B/10C owns the creation and its result/history; Phase 11 must target metadata by the operation's instrument ID
without reapplying the graph to an unrelated selection. The current headless and GUI paths are not equivalent.

### MCP direct example-patch application

`CAP-0084` uses [InstrumentBuildBridge::apply_example_patch](../../../crates/pertylizer/src/mcp_bridge/instrument_build.rs).
After catalog lookup, it either verifies the requested instrument exists or creates one. It calls the same
`SynthSession::apply_patch`, returning the instrument ID, patch name, module/connection counts and rendered
diagnostic lines. It does not queue the load tool's GUI payload. Existing-instrument application replaces the
graph rather than appending it; this helper does not rename an existing instrument to the patch name.

The shared applicator resets counters, clears the graph, then installs modules, parameters, scripts and
connections, and mirrors patch octave offset. It can return after failed clearing or continue after individual
failures; no previous-graph restoration follows. GUI-only visualizers and `SignalMonitor` are skipped without
an error entry, and this path does not restore GUI groups/layout/canvas metadata. Empty `errors` thus does not
prove complete catalog fidelity. The typed partial result preserves reported losses, but a count is not a DSP
completion acknowledgement. Phase 10A/10B owns canonical instantiation/replacement and explicit retained metadata;
10C and 11 own history/dirty state and GUI reconciliation. This does not approve a new wire contract.

`apply_patch_migrates_legacy_multislot_script` in `session.rs` checks that the shared applicator diagnoses a dropped
legacy second script slot while preserving slot 1. It does not call either MCP patch tool or exercise GUI target
selection. Source inspection here is not a runtime comparison of the two adapters.

### Cleanup, lint and example-patch migration checks still owed

| Capabilities | Owner | Observable check before `Verified` |
|---|---|---|
| All five | 10E adapter; 10A reads and 10B mutation results | Exercise direct and batched calls over supported transports, required/invalid inputs, output schemas and outcome metadata. Assert diagnostics and resulting state agree. Catalog wording must match implemented behavior; a successful report is not automatically an effect/completion receipt. |
| CAP-0178 | 10A references; 10B cleanup; 10C history; 10D assets | Declare retention roots, then prune a fixture with placed/unplaced content, manual-use instruments and referenced/orphan samples. Inject command-send failures and concurrent edits/imports. Removed stable IDs, retained state and diagnostics must agree, with no silent partial deletion; undo and dirty state must follow the operation. |
| CAP-0153 | 10A validation; 10E report | Check severity totals, info-only entries, orphan tracks, hidden events and instruments removed during inspection. Use disconnected source/output subgraphs and cycles to falsify unsupported coverage claims. State the inspected revision/coverage; error diagnostics in a returned report must remain distinguishable from inability to produce the report. |
| CAP-0156 | 10A catalog; 10E selectors | Match listing metadata/counts to definitions; test case variants, unknown and duplicate names under the declared selection policy. Verify the catalog read leaves project state unchanged. Each built-in definition still needs its own capability disposition and fidelity checks. |
| CAP-0171 | 10A content; 10B creation; 10C history; 10E result; 11 GUI | Keep instrument A selected while requesting a new patch instrument B; delay GUI drain and queue multiple loads. A must remain unchanged, metadata must reach B, and each accepted operation must retain its own outcome. Compare headless/GUI results and inject creation, application and GUI-delivery failures. |
| CAP-0084 | 10A content; 10B replacement; 10C history; 10E result; 11 GUI | Apply to an existing and a newly created instrument, preserving unrelated instruments. Check graph replacement, retained metadata, unknown IDs/names, skipped GUI-only content and application failures. Partial diagnostics, counts, retained graph, undo/dirty state and GUI reconciliation must agree. |

Only these five additional MCP entries become `Migrate`/`Classified`; twenty-one capability rows now have
supporting dispositions. No identifier is added: CAP-0510 remains next. P00B-T002, the other capability surfaces,
representative-path evidence (T004), round-trip verification (T005), and the Phase 0B exit remain open.

## MCP discovery 2026-09-12

This bounded P00B-T002 inspection classifies five discovery tools at `3fb94fb4` using the existing
[source-inspection method](#method-and-acceptance-boundary). Trace shipped dispatch to the actual source of each
answer; a false behavioral claim, missing route, inconsistent owner or untestable criterion blocks classification.
Optional implementation detail does not. The [roadmap](../ROADMAP.md) assigns node/port declarations and their
discovery surfaces to Phase 5, format/schema artifacts to 10D and the MCP adapter to 10E. The
[master plan](../master-plan.md#phase-5-declarative-node-and-parameter-api) makes the node declaration the single
source for UI/discovery metadata and derived catalogs; Phase 10E adapts those declarations to the wire. No new
node support, format, wire contract, compatibility break or ADR acceptance is authorized by `Migrate`.

The source and cited tests were read; a read-only JSON walk confirmed the existing script-parameter schema gap
and the version property's shape. No Rust test, GUI interaction, live MCP call or runtime experiment was executed.
This pass changes only the inventory, not production code or the evidence harness. Each underlying module type
still needs its own disposition and fidelity evidence; migrating discovery is not a claim that every V1 type is
implemented in V2.

### Discovery entry and output boundary

All five handlers in [tools/discovery.rs](../../../crates/synth_mcp/src/server/tools/discovery.rs) carry
`read_only_hint = true`. The default discovery router and the separate batch dispatch in
[server.rs](../../../crates/synth_mcp/src/server.rs) name them all; CAP-0040/0041 cover GUI HTTP and headless stdio.
They read catalog/schema sources rather than mutate a project. Four return `Json<T>` with typed output schemas:
listings use `Listing`, detail uses `ModuleTypeInfo`, and search uses `ModuleSearchResult`. The project-schema
tool deliberately returns a JSON document as text with no `outputSchema` or structured-content half; batch
preserves it as text. `NoParams` serves listing, ports and schema; detail and search use their own input structs.
Bridge failures become tool errors, except the noted best-effort hint and serialization paths below.

The default [output-schema tests](../../../crates/synth_mcp/src/server/tests/output_schema.rs) guard the schema
presence rule and its named prose exceptions, and check that omitted parameter fields are not required.
[Batch dispatch coverage](../../../crates/pertylizer/tests/mcp_batch_dispatch_coverage.rs) supplies invalid scalar
parameters to detect missing routes without executing handlers. These checks are not evidence that the five live
replies were exercised by this inspection, nor that catalog data matches every runtime or persisted value.

### MCP module-type listing

`CAP-0162` calls [list_module_types_brief](../../../crates/pertylizer/src/mcp_bridge/instruments.rs). The source is
[module_factory::ALL_MODULE_TYPES](../../../crates/pertylizer/src/module_factory.rs), derived from the `ModuleType`
enum in declaration order. Each row has the type prefix, display name, coarse category and `gui_only` from
`is_visualizer()`. This path does not build descriptors or instantiate modules. Visualizers remain listed even
though `add_module` refuses them over MCP. It does not enumerate live instrument modules, ports or parameters.

**Coverage limit:** enum inclusion alone does not prove factory support. The detailed paths require
`get_descriptor`; they can fail or skip an entry the brief listing still exposes if a future enum variant lacks
factory wiring. `every_module_type_has_a_descriptor` in `module_factory.rs` guards that relation, but was not
run here. A listed type or a false `gui_only` flag is not a promise that every host/path can create it. V2 listing
must derive from the declared supported catalog and state availability, without resurrecting unimplemented V1
nodes through this inventory classification.

### MCP module-type detail

`CAP-0140` trims the required `type_key` and rejects an empty value. The
[DiscoveryBridge implementation](../../../crates/pertylizer/src/mcp_bridge/discovery.rs) resolves it with
[parse_module_type](../../../crates/pertylizer/src/mcp_bridge.rs), checks registry membership and obtains a
factory descriptor. The parser accepts keys and name aliases, including separator-insensitive display names;
the argument is intentionally an open string, not a closed enum. Unknown types return `InvalidModuleType`.
The handler may add near-miss hints from the brief catalog; failure of that hint lookup omits the hint while
preserving the original error. It is not a substitute type selection.

[get_descriptor](../../../crates/pertylizer/src/module_factory.rs) constructs temporary default modules/effects
or visualizers to obtain their descriptors. [build_module_type_info](../../../crates/pertylizer/src/mcp_bridge/discovery_impl.rs)
then supplies input/output ports and value domains, parameter bounds/defaults/units/choice metadata, category
flow hints and the math oscillator's algorithm-parameter table. Choice metadata includes numeric index plus
stable choice ID and display name; it is not a declaration of the on-disk encoding. Flow hints are category-level
prose, not connection validation. Temporary construction is off the audio path, not an allocation-free read.

**Instance limit:** this is a default type descriptor. Script knobs added by
[build_script_descriptor](../../../crates/pertylizer/src/session.rs) belong to the installed instance program and
are absent here. Neither detail nor search establishes that all instance parameters are enumerated. The
[parser tests](../../../crates/pertylizer/src/mcp_bridge/tests/helpers.rs) check representative key/name aliases;
[math oscillator tests](../../../crates/pertylizer/tests/math_oscillator_algorithm_params.rs) use the shared search
builder to inspect algorithm labels and absence on another type. They do not call this MCP handler or validate
all descriptor values against engine behavior. Phase 5 owns declaration agreement; 10E exposes it through MCP.

### MCP module search

`CAP-0208` reaches [search_module_types](../../../crates/pertylizer/src/mcp_bridge/discovery_impl.rs). Optional
category/input/output filters are combined with AND. At the tool boundary categories are `voice`, `effect`,
`visualizer` and signal filters are `audio`, `control`, `gate`, `midi`; other spellings are rejected by enum
deserialization. Port filters require an actual port of that exact type/direction, not merely a compatible type.

A text query is split on whitespace and lowercased. Token scores add matches in name/key, tags, description and
parameter names, using substring matching with a one-trailing-character fallback for longer tokens. Not every
query token must match: a positive aggregate score admits the type. With text, higher scores precede lower ones
and ties preserve registry order; no/blank text keeps registry order under the hard filters. Descriptors missing
from the factory are skipped. A genuine text query with no matches may yield up to five near-miss names,
respecting the same hard filters; this remains a successful empty search, with a handler-supplied hint.

**Limit boundary:** the bridge constructs all matching detail records and records `total_matched`. Only then
does the handler truncate to the requested positive `limit`, default 20, adding a truncation hint. Zero is rejected;
there is no declared positive maximum or offset pagination. The limit bounds returned entries, not descriptor
construction or search work. Empty search results do not prove a capability is absent, and typed enum validation
of MCP arguments does not apply to callers invoking the string-based helper directly.

[search_modules.rs](../../../crates/pertylizer/tests/search_modules.rs) checks representative ring-modulator
queries/ranking, nonsense, typo suggestions, filter-respecting suggestions and blank queries through the helper.
[schema_enum.rs](../../../crates/synth_mcp/src/server/tests/schema_enum.rs) checks the MCP filter enums and invalid
spellings. The helper tests do not exercise handler truncation, `limit = 0`, live transport or reply-size bounds.

### MCP port-type discovery

`CAP-0167` builds its response directly in the server, without a bridge call. Four explicit entries describe
`audio`, `control`, `gate` and `midi`; `compatible_with` iterates
[PortType::ALL](../../../crates/synth_core/src/module_traits.rs) and applies `source.can_drive(destination)`.
Audio drives audio/control, control drives audio/control/gate, gate drives gate/control, and MIDI drives only
MIDI. This reports V1's signal-type relation, not graph admission, endpoint existence, per-port value-domain
compatibility or the V2 compiler's rules. Displayed ranges are descriptive hints, not clamps on sample values.

**Maintenance limit:** the entries and prose are hand-written. A debug-only length assertion compares them to
`PortType::ALL`; only compatibility lists are derived from it. A new type therefore needs an explicit row, and
length equality alone does not prove unique/correct entries. `port_type_ids_and_compatibility_are_stable` in
`module_traits.rs` checks representative pairs and identifiers, not the MCP result. Phase 5 owns the V2 port
contract and compiler rules; 10E must publish that contract without silently reusing V1's relation under changed
semantics. No port-protocol change is made here.

### MCP project-schema discovery

`CAP-0143` calls [InstrumentBridge::get_project_schema](../../../crates/pertylizer/src/mcp_bridge/instruments.rs).
`PROJECT_SCHEMA_JSON` is compiled in with `include_str!` from
[schemas/project.schema.json](../../../schemas/project.schema.json). It is parsed as `RawValue` to check JSON
syntax and retain the schema's original numeric text. The wrapper carries `schema_file`, `schema_format_version`
from `ProjectFile::FORMAT_VERSION` (currently `1.1`), and `app_version` from the serving build. The latter is not
proof of which build generated the artifact, despite the tool description's wording. It neither regenerates the
schema nor reads a live file or validates a caller's project. `to_json` formats the wrapper while retaining the
raw schema; its generic serialization-error fallback returns a text string inside an `Ok`, not a typed tool
failure. This inspection did not reproduce such a failure for this concrete payload.

**Artifact limit:** exact delivery does not prove a correct or complete schema. The version property is a string
without a `const`/`enum` restriction, so this artifact alone does not enforce version admission. The existing
IDN-0015 finding in the [identity inventory](identities.md) also remains visible in the inspected JSON: `script`
and `audio_script` have empty parameter-property maps with `additionalProperties: false`, whereas installed
programs can declare knobs. Such saved knob keys cannot satisfy those schema branches. Phase 10D owns the
versioned schema, conversions and rejection policy; 10E owns truthful publication and errors. This pass neither
repairs the existing schema nor chooses the V2 format or a new wire envelope.

`embedded_project_schema_is_valid_and_well_formed` in the bridge helper tests checks JSON/root structure and
nonempty build version, not byte equality or all schema semantics. In
[schemas_validate_examples.rs](../../../crates/pertylizer/tests/schemas_validate_examples.rs),
`checked_in_schemas_match_generated` compares generated artifacts with committed bytes, and
`example_files_validate_against_schemas` validates selected project/patch example files, excluding ZIP bundles.
Neither proves all possible instance-authored data is modeled, and those tests were not run in this inspection.

### Discovery migration checks still owed

| Capabilities | Owner | Observable check before `Verified` |
|---|---|---|
| All five | 10E MCP adapter over named declaration/format owners | Exercise direct and batch calls over supported transports; compare declared input/output schemas, typed versus text reply shapes, errors and catalog contents. Reads must leave canonical project/dirty state unchanged. Keep successful empty results distinct from unsupported input or failed discovery. Any approved wire change needs an explicit contract decision. |
| CAP-0162 | 5 declarations/catalog; 10E listing | Compare the complete supported registry with unique listed keys, categories and availability flags. Add a declaration without factory/discovery support and require an explicit failure or declared unavailability, not a silently misleading entry. Individual module migration remains separately gated. |
| CAP-0140 | 5 declarations/metadata; 10E detail | Exercise keys, aliases, whitespace, unknown types and hint-lookup failure. Compare ports, domains, defaults, choices and algorithm metadata with declarations. For instance-declared script knobs, state the type/instance boundary and verify the instance discovery route before claiming complete parameter coverage. |
| CAP-0208 | 5 declarations/catalog; 10E search | Check ANDed filters, exact port types, mixed matching/nonmatching tokens, stable score ties, blank queries, typos and zero matches. Test default/zero/large limits and pre-truncation totals. Declare and measure work/reply bounds before treating the response limit as a resource bound. |
| CAP-0167 | 5 port/compiler contract; 10E catalog | Compare every source/destination pair and every unique published type with the supported declaration and connection validators. Test wrong direction, missing endpoints and incompatible value domains separately so type compatibility is never mistaken for complete connection admission. |
| CAP-0143 | 10D schema/versioning; 10E publication | Compare delivered raw schema content on direct/batch paths with the selected artifact and verify truthful version provenance. Check malformed artifacts, serialization failure handling, unsupported format versions and instance-declared script parameters. Validate applicable fixtures, while keeping artifact fidelity distinct from schema correctness. |

Only these five existing rows gain a disposition. Twenty-six capability rows, including fourteen MCP tools, now have
supporting classification. CAP-0510 remains next; other capabilities, per-module fidelity, T004/T005 evidence and
the complete P00B-T002/Phase 0B exit remain open.

## Audit passes

| Date       | Source revision | Discovery method                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Coverage/result                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Evidence                           |
|------------|-----------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------|
| 2026-08-12 | `dd69b657`      | MCP tools counted by parsing `#[tool(...)]` attributes and their following `async fn` in `crates/synth_mcp/src/server/tools/*.rs` and `server.rs` (219 unique, per-module breakdown recorded above). `EngineCommand`/`EngineEvent`/`ModuleType` counted with a brace-depth-aware top-level variant parser. Built-in patches counted from `pub use …::patch_*` in `crates/pertylizer/src/patches/mod.rs`. Group templates read from `categorized_group_templates()`. CLI read from `crates/pertylizer/src/main.rs`. OSC addresses counted in `crates/synth_osc_protocol/src/lib.rs`. GUI surfaces enumerated by module listing, **not** by action. | 47 entries; all four seeds reproduce. Known gaps: GUI capabilities are listed per view rather than per action/menu/shortcut, so no GUI *action* is yet individually classified; no entry has a disposition; per-item rows for the 219 tools, 76 commands, 75 module types, and 68 patches are pass-2 work.                                                                                                                                                                                                                                                                                                                            | Pending `EVD` record for P00B-T002 |
| 2026-08-12 | `dd69b657`      | `AppShortcut::ALL` read from `gui/shortcuts.rs` (a closed 7-element table that also renders the menu); MCP annotation attributes counted by `rg -o` over `read_only_hint`/`destructive_hint`/`idempotent_hint`; `dialog_flow.rs` read for the startup ordering.                                                                                                                                                                                                                                                                                                                                                                                   | 8 entries added (`CAP-0048`..`CAP-0055`); `CAP-0011` upgraded to `Classified` once the read/mutate split was separated — annotation coverage turned out to be complete. Gaps remained deliberate at that pass: no entry yet had a disposition, and CAP-0017's external use was unknown. Current correction: CAP-0017 is a public Rust surface even without a workspace caller, and proposed ADR-0039 supplies an explicit disposition for independent review. | Pending `EVD` record for P00B-T002 |
| 2026-08-25 | `c075ef10` | The shipping CPAL 0.18.1 output/input callbacks at this revision and the candidate CPAL 0.18.2 registry source identified by the updated `Cargo.lock` checksum were read. Every 0.18.2 `ErrorKind` was enumerated, and the stderr-only baseline consumer was traced before this change replaced it with an atomic handoff. | Added `CAP-0508`. The category labels preserve CPAL 0.18.2's richer distinction; the replacement callback path is allocation-, lock-, and logging-free, while non-real-time GUI/MCP polling surfaces coalesced diagnostics and the row leaves durable structured delivery as Phase 9 work. | `Cargo.lock`; `cpal_backend.rs`; EVD-0016; independent uncommitted review |
| 2026-09-09 | `cf2edf5b` | Followed six document/history shortcuts and their menu handlers, plus startup recovery, into GUI project/dialog/history/autosave flows and their project/store callees; read the named existing tests without running them. | Seven rows (`CAP-0048`–`CAP-0053`, `CAP-0055`) assigned `Migrate` and `Classified`; V2 owners and pending checks named. Corrected the all-menu-bindings claim and separated project recovery from recording-take semantics. Other rows retain their previous status; P00B-T002 remains incomplete. | [Source inspection and limits](#project-actions-2026-09-09) |
| 2026-09-11 | `85d92f1b` | Followed `Cli`, `Command`, `RenderArgs` and `CompareArgs` through runtime mode dispatch, GUI/stdio startup and render/compare library callees; used the existing source-inspection method and read the cited tests without running them. | Classified CAP-0040–CAP-0043 and new CAP-0509 as `Migrate`; corrected the render-option count and top-level OSC-switch scope, recorded null-backend headless behavior and I/O/shutdown limits. Twelve rows now have dispositions and supporting classification; other rows are unchanged. P00B-T002 remains incomplete. | [CLI source inspection and limits](#cli-entry-points-2026-09-11) |
| 2026-09-11 | `bab84945` | Used the existing source-inspection method to follow four project-tool handlers through direct/batch dispatch, ProjectBridge, project/sample mutation, save builders and GUI notification consumers; read the cited tests without running them. | CAP-0172, CAP-0173, CAP-0205 and CAP-0206 are `Migrate`/`Classified`, with source limits and pending migration checks. Recorded the catalog/patch-refusal discrepancy, nontransactional load/reset and save capture limits; corrected the project-tool rollup. Sixteen rows now have supporting dispositions; P00B-T002 remains incomplete. | [MCP project-operation source inspection](#mcp-project-operations-2026-09-11) |
| 2026-09-12 | `ee5c6600` | Followed five default MCP handlers and batch routes through cleanup, lint aggregation, built-in catalog lookup, shared patch application and the GUI pending-patch consumer; read the named tests without running them. | Classified CAP-0084, CAP-0153, CAP-0156, CAP-0171 and CAP-0178 with owners and pending checks. Recorded silent cleanup failures, lint coverage limits and the load/apply diagnostic and GUI-target differences. Twenty-one rows have supporting dispositions; P00B-T002 remains incomplete. | [Cleanup, lint and example-patch inspection](#mcp-cleanup-lint-and-example-patches-2026-09-12) |
| 2026-09-12 | `3fb94fb4` | Followed five discovery handlers and batch routes into enum/factory catalogs, descriptor builders, search filters/ranking, port compatibility and the embedded schema; inspected JSON branches and read named tests without running Rust. | Classified CAP-0140, CAP-0143, CAP-0162, CAP-0167 and CAP-0208 with V2 owners and pending checks. Recorded type/instance limits, post-build search truncation and schema publication/correctness limits. Twenty-six rows have supporting dispositions; P00B-T002 remains incomplete. | [Discovery inspection](#mcp-discovery-2026-09-12) |

Completion requires each discovered entry to have reachability, disposition, V2 ownership, and verification. Matching
the seed counts alone is insufficient.
