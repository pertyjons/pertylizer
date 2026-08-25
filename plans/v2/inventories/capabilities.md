# Capability and Reachability Inventory

| Field         | Value      |
|---------------|------------|
| Status        | Active     |
| Phase         | 00B        |
| Last reviewed | 2026-08-12 |

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

Entries use `CAP-NNNN` identifiers. Next free identifier: `CAP-0509`.

Passes 1 and 2 were a **surface census**; pass 3 added the per-item enumeration the master plan requires. No entry in
this ledger has a disposition yet.

**Status rule.** The [register vocabulary](README.md) defines `Classified` as required fields *and* disposition filled
with supporting evidence. Because no disposition has been assigned, **no entry here may be `Classified`**, however well
understood it is. Entries whose facts are settled but whose disposition is open stay `Discovered` or `Investigating`.
Assigning dispositions is the remaining work of P00B-T002, not a documentation formality.

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
| CAP-0009 | MCP     | Project tools — 5 (`project.rs`): new, load, save, save patch, lint                                                                                                  | Same                                                                 |             |                      | Save path is `STATE-0027`/`STATE-0031` sensitive                                                                                                                                                                                                | Investigating |
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
| CAP-0040 | CLI        | `pertylizer` (no subcommand) — launch the GUI                                                                                                                                      | Single `[[bin]]`             |             |                      |                                                                                          | Discovered    |
| CAP-0041 | CLI        | `pertylizer --headless` — MCP server on stdio                                                                                                                                      | `mcp` feature                |             |                      |                                                                                          | Discovered    |
| CAP-0042 | CLI        | `pertylizer render` — 11 arguments incl. `--protocol-version`, `--bit-depth`, `--solo-track`/`--mute-track` (accept id **or unique name**), `--result-json`                        | Any build                    |             |                      | Contract carries its own `PROTOCOL_VERSION`; open work in `plans/TODO.md` §5.6           | Investigating |
| CAP-0043 | CLI        | `--no-osc` global flag                                                                                                                                                             | `osc` feature                |             |                      |                                                                                          | Discovered    |
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
| CAP-0048 | GUI action   | `New` project — `Cmd/Ctrl+N`                                                                     | Menu and shortcut, one dispatch table (`gui/shortcuts.rs`)                         |             |                      |                                                               | Discovered    |
| CAP-0049 | GUI action   | `Open` project — `Cmd/Ctrl+O`                                                                    | Same                                                                               |             |                      |                                                               | Discovered    |
| CAP-0050 | GUI action   | `Save` — `Cmd/Ctrl+S`, prompts for a path when the project has none                              | Same                                                                               |             |                      | Carries the layout overlay (`STATE-0027`)                     | Discovered    |
| CAP-0051 | GUI action   | `Save As` — `Shift+Cmd/Ctrl+S`                                                                   | Same                                                                               |             |                      | Moves the path a recovery snapshot is keyed by (`STATE-0057`) | Discovered    |
| CAP-0052 | GUI action   | `Undo` — `Cmd/Ctrl+Z`                                                                            | Same                                                                               |             |                      | 60 action variants; see `LIMIT-0063`, `IDN-0027`              | Discovered    |
| CAP-0053 | GUI action   | `Redo` — `Shift+Cmd/Ctrl+Z`                                                                      | Same                                                                               |             |                      |                                                               | Discovered    |
| CAP-0054 | GUI action   | `Toggle playback` — `Space`, gated so it never fires while text is focused                       | Same                                                                               |             |                      |                                                               | Discovered    |
| CAP-0055 | GUI workflow | Recovery offer at startup — decides which document the session opens in, before any other dialog | `gui/egui_backend/dialog_flow.rs`, only when a snapshot supersedes the manual save |             |                      | See `STATE-0054`; ADR-0024                                    | Investigating |

`AppShortcut::ALL` is a closed 7-element table and the same source renders the menu binding it dispatches, so this
enumeration is complete for app-level actions by construction rather than by search.

## Per-item enumerations

The master plan requires this ledger to name **every** `EngineCommand` and `EngineEvent` variant, every MCP tool with
its read/mutate behavior, and every module type, built-in patch, and group template — not a family count. The rows
below are that enumeration, generated from source at `dd69b657` by the method recorded in the pass-3 audit row, so a
capability added or removed later shows up as a diff rather than as a changed total.

`CAP-0001`..`CAP-0010`, `CAP-0014`, `CAP-0015`, `CAP-0023`, and `CAP-0027` remain at their stable identifiers as
**rollup rows**: they describe a surface, carry no disposition of their own, and are not counted as capability entries.
The authoritative per-capability entries are below. Every one is `Discovered`: reachability is filled, disposition is
not, and per the register vocabulary an entry without a disposition cannot be `Classified`.

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
| CAP-0084 | MCP | `apply_example_patch` | `instruments.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
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
| CAP-0140 | MCP | `get_module_type_info` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0141 | MCP | `get_note_graph` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0142 | MCP | `get_parameter` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0143 | MCP | `get_project_schema` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0144 | MCP | `get_sample_info` | `samples.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0145 | MCP | `get_sampler_state` | `samples.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0146 | MCP | `get_song_info` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0147 | MCP | `get_tempo_map` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0148 | MCP | `get_ui_snapshot` | `instruments.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0149 | MCP | `get_version` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0150 | MCP | `get_yams_reference` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0151 | MCP | `import_sample` | `samples.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0152 | MCP | `insert_module_between` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0153 | MCP | `lint_project` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0154 | MCP | `list_arrangement` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0155 | MCP | `list_automation_lanes` | `automation.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0156 | MCP | `list_example_patches` | `instruments.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0157 | MCP | `list_input_devices` | `audio_input.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0158 | MCP | `list_instruments` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0159 | MCP | `list_master_effects` | `mixing.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0160 | MCP | `list_mod_graphs` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0161 | MCP | `list_mod_targets` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0162 | MCP | `list_module_types` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0163 | MCP | `list_modules` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0164 | MCP | `list_note_graphs` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0165 | MCP | `list_notes` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0166 | MCP | `list_patterns` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0167 | MCP | `list_port_types` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0168 | MCP | `list_return_busses` | `mixing.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0169 | MCP | `list_samples` | `samples.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0170 | MCP | `list_tracks` | `sequencer.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0171 | MCP | `load_example_patch` | `instruments.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0172 | MCP | `load_project` | `project.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0173 | MCP | `new_project` | `project.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0174 | MCP | `normalize_sample` | `samples.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0175 | MCP | `note_off` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0176 | MCP | `note_on` | `instruments.rs` | mutating | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0177 | MCP | `offset_automation_lane` | `automation.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0178 | MCP | `optimize_project` | `project.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
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
| CAP-0205 | MCP | `save_patch` | `project.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0206 | MCP | `save_project` | `project.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0207 | MCP | `scale_automation_lane` | `automation.rs` | destructive | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
| CAP-0208 | MCP | `search_modules` | `discovery.rs` | read | `synth` server: HTTP `127.0.0.1:9850/mcp` and `--headless` stdio | | | Discovered |
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

Next free identifier after this section: `CAP-0509`.

## Audit passes

| Date       | Source revision | Discovery method                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  | Coverage/result                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       | Evidence                           |
|------------|-----------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------|
| 2026-08-12 | `dd69b657`      | MCP tools counted by parsing `#[tool(...)]` attributes and their following `async fn` in `crates/synth_mcp/src/server/tools/*.rs` and `server.rs` (219 unique, per-module breakdown recorded above). `EngineCommand`/`EngineEvent`/`ModuleType` counted with a brace-depth-aware top-level variant parser. Built-in patches counted from `pub use …::patch_*` in `crates/pertylizer/src/patches/mod.rs`. Group templates read from `categorized_group_templates()`. CLI read from `crates/pertylizer/src/main.rs`. OSC addresses counted in `crates/synth_osc_protocol/src/lib.rs`. GUI surfaces enumerated by module listing, **not** by action. | 47 entries; all four seeds reproduce. Known gaps: GUI capabilities are listed per view rather than per action/menu/shortcut, so no GUI *action* is yet individually classified; no entry has a disposition; per-item rows for the 219 tools, 76 commands, 75 module types, and 68 patches are pass-2 work.                                                                                                                                                                                                                                                                                                                            | Pending `EVD` record for P00B-T002 |
| 2026-08-12 | `dd69b657`      | `AppShortcut::ALL` read from `gui/shortcuts.rs` (a closed 7-element table that also renders the menu); MCP annotation attributes counted by `rg -o` over `read_only_hint`/`destructive_hint`/`idempotent_hint`; `dialog_flow.rs` read for the startup ordering.                                                                                                                                                                                                                                                                                                                                                                                   | 8 entries added (`CAP-0048`..`CAP-0055`); `CAP-0011` upgraded to `Classified` once the read/mutate split was separated — annotation coverage turned out to be complete. Gaps remained deliberate at that pass: no entry yet had a disposition, and CAP-0017's external use was unknown. Current correction: CAP-0017 is a public Rust surface even without a workspace caller, and proposed ADR-0039 supplies an explicit disposition for independent review. | Pending `EVD` record for P00B-T002 |
| 2026-08-25 | `c075ef10` | The shipping CPAL 0.18.1 output/input callbacks at this revision and the candidate CPAL 0.18.2 registry source identified by the updated `Cargo.lock` checksum were read. Every 0.18.2 `ErrorKind` was enumerated, and the stderr-only baseline consumer was traced before this change replaced it with an atomic handoff. | Added `CAP-0508`. The category labels preserve CPAL 0.18.2's richer distinction; the replacement callback path is allocation-, lock-, and logging-free, while non-real-time GUI/MCP polling surfaces coalesced diagnostics and the row leaves durable structured delivery as Phase 9 work. | `Cargo.lock`; `cpal_backend.rs`; EVD-0016; independent uncommitted review |

Completion requires each discovered entry to have reachability, disposition, V2 ownership, and verification. Matching
the seed counts alone is insufficient.
