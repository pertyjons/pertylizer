# Persisted-State Ownership Inventory

| Field         | Value                  |
|---------------|------------------------|
| Status        | Active                 |
| Phase         | 00B                    |
| Last reviewed | 2026-08-12             |

This ledger records every field currently saved, reconstructed, mirrored, or used as a dirty/undo signal, then assigns
one intended V2 state partition.

## V2 ownership classes

- `Project document` — authored state, including intentionally persisted
  `EditorMetadata`;
- `Runtime session` — transport, focus/selection, preview, recording, active
  device connection, and compile/plan coordination;
- `User settings` — persisted per-user preferences outside the project;
- `Frontend-local transient` — hover, scroll caches, open dialogs, temporary
  input, drag state, and comparable view-local state;
- `Host/service configuration` — deployment, protocol, authorization, feature,
  and default resource policy;
- `Runtime job` — revision-pinned render, export, analysis, or conversion work;
- `Runtime telemetry` — lossy observation and counters;
- `Removed` — a current field or mirror with no V2 equivalent.

These classes are mutually exclusive. `EditorMetadata` is not a peer of the
project document: it is the persisted presentation-intent section inside it.
Frontend-local presentation state is a separate non-persisted class.

## How to read the `Dirty/undo behavior` column

This column is filled from `crates/pertylizer/src/dirty.rs` and the 60 `UndoAction` variants in
`crates/pertylizer/src/undo.rs`. Dirty state is not a flag that editors set; it is seven observers compared against a
baseline captured at load/save:

| Term | Kind | Covers |
|------|------|--------|
| `song` | counter (`SharedSong::revision`) | notes, patterns, placements, tempo, time signatures, automation, Note/Mod Grid graphs, tracks, mixer controls, return buses and sends |
| `graph` | counter (`SharedGraph::version`) | instruments, modules, parameters, connections |
| `samples` | counter (`SampleLibrary::revision`) | sample import/delete/rename/metadata and destructive edits |
| `ui` | counter, explicitly reported via `mark_dirty()` | instrument properties and project metadata held in `InstrumentUiState` |
| `layout` | **fingerprint** | module positions, group boxes and their persisted fields |
| `global` | **fingerprint** | master volume, keyboard octave, glide, transport loop, master and return-bus effect chains |
| `effect_order` | **fingerprint** | the order of each instrument's effect chain |

The three fingerprints exist because their state is reached from too many places for "remember to report" to hold —
dragging a module never marked the project dirty until `layout` was derived from the data itself.

`SynthApp::is_dirty` also treats the project as clean when the undo stack is back at its saved depth, but only until
`observe_untracked_mutation` sees a revision move that the undo manager did not record; after that the counters answer
for the rest of the session. So a `Dirty` entry below is reliable; an `Undo` entry of `none` is what disables the
stack-depth shortcut.

## Ledger

Entries use `STATE-NNNN` identifiers. Next free identifier: `STATE-0061`.

Uniform collections are recorded at **section granularity**: one entry per persisted structure, not one per element of
a repeated record. Where a single field carries a distinct migration question of its own, it gets its own entry.
`Intended V2 owner` is proposed only where the class is unambiguous from the field's meaning; the contested ones are
left blank because they *are* ADR-0013 and ADR-0018.

**Status rule.** The [register vocabulary](README.md) defines `Classified` as required fields *and* disposition filled
**with supporting evidence**. The `Evidence` column is blank for every entry here — the audits are not yet written up
as `EVD` records — so **no entry may be `Classified` yet**, including the ones whose V2 owner is obvious. They become
`Classified` when the evidence records land, not before.

### Project document — root and envelope

| ID | Field/state | Domain type | Current owner | Mirrors/save sources | Dirty/undo behavior | Intended V2 owner | Migration | Evidence | Status |
|----|-------------|-------------|---------------|----------------------|---------------------|-------------------|-----------|----------|--------|
| STATE-0001 | `file_type` | `String`, always `"project"` | `ProjectFile` | Written by every save path | Dirty: never — a constant. Undo: N/A | Project document | Replaced by the Format V2 envelope | | Investigating |
| STATE-0002 | `version` | `String`, `ProjectFile::FORMAT_VERSION` = `"1.1"` | `ProjectFile` | Every save path | Dirty: never — a constant. Undo: N/A | Project document | | | Investigating |
| STATE-0003 | `author` | `Option<Author>` (name, email, website, license) | `SynthApp::current_project_author` — a **per-project** field, not the settings value | **Three holders.** The GUI edits `current_project_author` in Project → Edit metadata (`egui_backend.rs:4367-4397`) and `build_save_options` writes it, mapping empty to `None` (`project_flow.rs:569-573`). Load overwrites it from `project.author`, falling back to `settings.author` only when the file carries none (`project_flow.rs:758-760`). MCP keeps its own slot (STATE-0060), mirrored from the loaded project. `AppSettings.author` is the **seed for a new project only** (`egui_backend.rs:608`). | Dirty: `ui` (project metadata). Undo: **none** | Project document | The three holders must not drift; nothing reconciles the GUI field with the MCP slot today | | Investigating |
| STATE-0004 | `active_instrument_id` | `u64` | `ProjectFile` | GUI selection, written straight onto the built project (`project_flow.rs:560`); `ProjectBuildOptions.active_instrument_id` on the MCP path, where `None` falls back to "first instrument built" | Dirty: **not observed by any of the seven terms** — the three instrument-switch sites (`egui_backend.rs:1348`, `:3669`, `:3877`) set it without calling `mark_dirty()`, yet every save writes it. Undo: none | | Classification contested: this is editor focus persisted inside the document | | Investigating |
| STATE-0005 | `instruments[]` | `Vec<InstrumentState>` | `ProjectFile` | Engine snapshots via `build_project_from_engine`, overlaid with GUI `PatchEditor` metadata | Dirty: `graph`. Undo: `SetRackModules` (a whole-rack snapshot with severed connections and a direction flag); **no dedicated instrument add/remove action** | Project document | | | Investigating |
| STATE-0006 | `song` | `Song` | `SharedSong` (`RwLock` + `ArcSwap`) | Read from `SharedSong` at save | Dirty: `song`. Undo: broad — see the song rows below | Project document | | | Investigating |

### Project document — global state

| ID | Field/state | Domain type | Current owner | Mirrors/save sources | Dirty/undo behavior | Intended V2 owner | Migration | Evidence | Status |
|----|-------------|-------------|---------------|----------------------|---------------------|-------------------|-----------|----------|--------|
| STATE-0007 | `global.master_volume` | `Gain` | Engine master bus | The authoritative copy. `patch.settings.master_volume` also exists but is a constant on this path — see STATE-0035. | Dirty: `global` fingerprint. Undo: `SetMasterVolume` | | | | Investigating |
| STATE-0008 | `global.octave_offset` | `i32` | GUI keyboard | The authoritative copy for a project. `session.rs:1372` *also* applies `patch.settings.octave_offset` on load, which in a project is always the default `0` — so the two paths can fight. Keyboard octave is a session/UI concern stored in the document. | Dirty: `global` fingerprint. Undo: **none** | | | | Investigating |
| STATE-0009 | `global.glide_time` | `Seconds` | Engine | The authoritative copy; the per-patch copy is a constant — see STATE-0035. | Dirty: `global` fingerprint. Undo: **none** | | | | Investigating |
| STATE-0010 | `global.return_bus_effects[]` | `Vec<ReturnBusEffectsState>` | Engine return buses | Reconstructed from engine at save. The bus *identity and fader* live in `song.return_busses`; only the effect chain lives here — one concept split across two sections of the file. | Dirty: `global` fingerprint (it hangs off `EngineState`, so the `graph` counter never saw it — this term had to be added after the fact). Undo: `SetChainEffect`, `SetChainEffectParameter`, `SetChainEffectBypass` | Project document | | | Investigating |
| STATE-0011 | `global.master_effects[]` | `Vec<ModuleState>`, chain order = array order | Engine master bus | Reconstructed from engine at save | Dirty: `global` fingerprint — adding a master effect is the documented case that reported clean before this term existed. Undo: same three chain actions | Project document | | | Investigating |

### Project document — per instrument

| ID | Field/state | Domain type | Current owner | Mirrors/save sources | Dirty/undo behavior | Intended V2 owner | Migration | Evidence | Status |
|----|-------------|-------------|---------------|----------------------|---------------------|-------------------|-----------|----------|--------|
| STATE-0012 | `instruments[].id` | `InstrumentId` | Engine | Engine snapshot | Dirty: `graph`. Undo: N/A — identity, not a value | Project document | | | Investigating |
| STATE-0013 | `instruments[].name`, `.description`, `.color` | `String`, `String`, `Option<String>` | Engine/GUI | Engine snapshot | Dirty: `ui` (instrument properties). Undo: **none** | Project document | | | Investigating |
| STATE-0014 | `instruments[].category` | integer-encoded enum | GUI / MCP `set_instrument_category` | Engine snapshot | Dirty: `ui`. Undo: **none** | Project document | See `IDN-0014` | | Investigating |
| STATE-0015 | `instruments[].channel` | MIDI channel selection | Engine | Engine snapshot | Dirty: `ui`. Undo: `SetInstrumentSettings` | Project document | | | Investigating |
| STATE-0016 | `instruments[].key_range` | array | Engine | Engine snapshot; also settable by MIDI learn (`EngineEvent::KeyRangeLearned`) | Dirty: `ui`. Undo: `SetInstrumentSettings`. Whether the MIDI-learn path pushes an undo entry is **not** established | Project document | | | Investigating |
| STATE-0017 | `instruments[].max_voices`, `.allocation_mode`, `.stealing_strategy` | voice-allocator config | Engine `AllocatorConfig` | Engine snapshot | Dirty: `ui`. Undo: `SetInstrumentSettings` | Project document | Interacts with the V2 admission model; `max_voices` is silently clamped (`LIMIT-0056`) | | Investigating |
| STATE-0018 | `instruments[].oversampling` | integer factor | Engine | Engine snapshot | Dirty: `ui`. Undo: `SetInstrumentSettings` | | Render-quality setting: document or session is undecided | | Investigating |
| STATE-0019 | `instruments[].muted`, `.solo` | `bool` | Engine mixer | Engine snapshot | Dirty: `ui`. Undo: `SetInstrumentSettings` | | Mute is authored; solo is arguably a transient monitoring state that is currently persisted | | Investigating |
| STATE-0020 | `instruments[].volume`, `.pan` | `NormalizedValue`, `BipolarValue` | Engine mixer | Engine snapshot | Dirty: `ui`. Undo: `SetInstrumentSettings` | Project document | | | Investigating |
| STATE-0021 | `instruments[].transpose`, `.unison_detune`, `.unison_spread` | `Semitones`, numbers | Engine | Engine snapshot | Dirty: `ui`. Undo: `SetInstrumentSettings` | Project document | A third, instrument-level unison concept distinct from `LIMIT-0028`'s two | | Investigating |
| STATE-0022 | `instruments[].velocity_amp_sensitivity`, `.velocity_filter_sensitivity` | numbers | Engine | Engine snapshot | Dirty: `ui`. Undo: `SetInstrumentSettings` | Project document | | | Investigating |
| STATE-0023 | `instruments[].sidechain_source_id` | `Option<integer>` | Engine | Engine snapshot | Dirty: `ui`. Undo: **none** | Project document | See `IDN-0013` | | Investigating |

### Project document — patch

| ID | Field/state | Domain type | Current owner | Mirrors/save sources | Dirty/undo behavior | Intended V2 owner | Migration | Evidence | Status |
|----|-------------|-------------|---------------|----------------------|---------------------|-------------------|-----------|----------|--------|
| STATE-0024 | `patch.name`, `.version`, `.description`, `.notes`, `.tags`, `.color`, `.author` | patch metadata | `Patch` | GUI patch editor; also written standalone to `.json` patch files | Dirty: `ui`. Undo: **none** | Project document | Shared-patch semantics are ADR-0011 | | Investigating |
| STATE-0025 | `patch.modules[]` | `Vec<ModuleState>` (75-variant enum) | Engine graph, mirrored in `shared_graph` | Engine reconstruction; `shared_graph` mirrors commands **by id**, so it is not a valid oracle for engine state | Dirty: `graph`. Undo: `SetRackModules` | Project document | | | Investigating |
| STATE-0026 | `module.parameters` | open map, keys are `ParamId` strings | Engine module | Engine snapshot | Dirty: `graph`. Undo: `SetModuleParameter`, merged within a 600 ms gesture window (`LIMIT-0065`) | Project document | Key set and unknown-key policy are ADR-0016 | | Investigating |
| STATE-0027 | `module.position` | `Position { x, y }` | **GUI `PatchEditor` only** | Present in the project **only** via the GUI overlay. `build_project_from_engine` writes `Position::default()`. | Dirty: `layout` fingerprint — derived from the data precisely because the canvas has ~30 mutation points and none reported themselves. Undo: **none for a drag**; a position only rides along inside a `SetRackModules` snapshot | Project document (`EditorMetadata`) | | | Investigating |
| STATE-0028 | `module.description` | `String` | GUI / MCP `set_module_description` | Engine snapshot | Dirty: `graph` or `ui` — **not established which**. Undo: **none** | Project document | | | Investigating |
| STATE-0029 | `module.scripts` | YAMS sources per module | Engine / script host | Engine snapshot | Dirty: `graph`. Undo: **none** — editing a YAMS script is not an undoable action | Project document | Script state identity is ADR-0008; `IDN-0029` makes the script's PRNG stream depend on the module's instance number | | Investigating |
| STATE-0030 | `patch.connections[]` | `Vec<ConnectionState>` | Engine graph | Engine reconstruction | Dirty: `graph`. Undo: `AddConnection`, `RemoveConnection`, plus the `severed` list inside `SetRackModules` | Project document | See `IDN-0019` | | Investigating |
| STATE-0031 | `patch.groups[]` | `Vec<ModuleGroupState>` — name, color, position, collapsed, members, exposed ports | **GUI `PatchEditor` only** | GUI overlay only; lost on the engine fallback path | Dirty: `layout` fingerprint. Undo: **none** | Project document (`EditorMetadata`) | | | Investigating |
| STATE-0032 | `patch.settings.canvas_size` | `Option<CanvasSize>` | **GUI `PatchEditor` only** | GUI overlay only | Dirty: `layout` fingerprint. Undo: **none** | Project document (`EditorMetadata`) | | | Investigating |
| STATE-0033 | `patch.settings.effect_chain_order` | ordered module-id strings | Engine effect chain | Engine reconstruction (`project_apply.rs:760`) | Dirty: `effect_order` fingerprint — its own term, added after the fact for the same reason as `layout`. Undo: `SetEffectChainOrder` | Project document | See `IDN-0020` | | Investigating |
| STATE-0034 | `patch.settings.bpm` | `Option<Bpm>` | Patch | **Written as a constant on the project save path.** `build_project_from_engine` starts each patch from `PatchSettings::default()` and fills only `effect_chain_order`, so every instrument in a saved `.ptz` carries `bpm: 120.0` whatever the song's tempo is. | Dirty: no term watches it, correctly — nothing in a project can change it. Undo: N/A | | Live only in standalone `.json` patch files; dead weight inside a project | | Investigating |
| STATE-0035 | `patch.settings.master_volume`, `.octave_offset`, `.glide_time` | duplicates of STATE-0007..0009 | Patch | Same as STATE-0034 — constants (`0.8`, `0`, `0`) on the project save path. Built-in and hand-authored **patch files** do set `octave_offset` (e.g. `patches/sub_bass.rs:154`), and `session.rs:1372` applies it on load. | Dirty: no term watches them; nothing in a project can change them. Undo: N/A | | Not a live duplicate inside a project, but loading a project still applies the default `octave_offset = 0` through the patch path | | Investigating |

### Project document — song

| ID | Field/state | Domain type | Current owner | Mirrors/save sources | Dirty/undo behavior | Intended V2 owner | Migration | Evidence | Status |
|----|-------------|-------------|---------------|----------------------|---------------------|-------------------|-----------|----------|--------|
| STATE-0036 | `song.name`, `.author`, `.description` | `String` | `SharedSong` | MCP `set_song_*`, GUI | Dirty: `song`. Undo: **none** | Project document | `song.author` is a bare `String` while `ProjectFile.author` is an `Author` struct — two author fields of different shape in one file, on top of the three holders in STATE-0003 | | Investigating |
| STATE-0037 | `song.default_tempo`, `.default_time_signature`, `.row_resolution` | `Bpm`, `TimeSignature`, `RowResolution` | `SharedSong` | GUI, MCP | Dirty: `song`. Undo: `SetTempo`, `SetTimeSignature`; `row_resolution` has **none** | Project document | | | Investigating |
| STATE-0038 | `song.tempo_changes[]`, `.time_signature_changes[]` | tempo map | `SharedSong` | GUI, MCP `set_tempo_at` | Dirty: `song`. Undo: `SetTempo`, `MoveTempo`, `SetTimeSignature` | Project document | | | Investigating |
| STATE-0039 | `song.tracks[]` | `Vec<SequencerTrack>` — id, name, description, color, instrument, mode, volume, pan, mute, solo, sends | `SharedSong` | GUI, MCP | Dirty: `song`. Undo: `AddTrack`, `DeleteTrack`, `RenameTrack`, `SetTrackMixer`, `SetTrackSend`; `description`, `color`, `mode` have **none** | Project document | Track/source/channel ownership is ADR-0034 | | Investigating |
| STATE-0040 | `song.patterns[]` | `Vec<Pattern>` — notes, automation, processors, note graph, `next_note_id` | `SharedSong` | GUI, MCP | Dirty: `song`. Undo: the largest group — 17 note actions, 6 automation actions, `AddPattern`, `DeletePattern`, `RenamePattern`, `SwapPattern`, `SetPatternLength`, `FreezePattern`, `RestorePattern`, `SetPatternNoteGraph`. **`IDN-0027`: undoing a note deletion allocates a new `NoteId`** | Project document | | | Investigating |
| STATE-0041 | `song.arrangement[]` | `Vec<PatternPlacement>` | `SharedSong` | GUI, MCP `place_pattern` | Dirty: `song`. Undo: `InsertPlacement`, `RemovePlacement`, `MovePlacement`, `SetPlacementLength`, `SetPlacementLoopMode` | Project document | | | Investigating |
| STATE-0042 | `song.sections[]` | `Vec<ArrangementSection>` — id, name, kind, color, start, length | `SharedSong` | GUI | Dirty: `song`. Undo: `SetArrangementSections` (whole-list snapshot) | Project document | | | Investigating |
| STATE-0043 | `song.return_busses[]` | `Vec<ReturnBus>` — id, name, description, color, volume, pan, mute, solo, sends | `SharedSong` | GUI, MCP | Dirty: `song`. Undo: `SetReturnBus`, `SetReturnBusMixer`, `SetReturnSend` | Project document | Effect chain lives in `global.return_bus_effects` — see STATE-0010 | | Investigating |
| STATE-0044 | `song.mod_graphs[]`, `.note_graphs[]` | graph pools with node positions and descriptions | `SharedSong` | GUI, MCP | Dirty: `song`. Undo: `SetModGraph`, `SetNoteGraph` (whole-graph snapshots, so node positions *are* undoable here — unlike patch module positions) | Project document | Node positions here **are** persisted, unlike patch module positions (STATE-0027) | | Investigating |
| STATE-0045 | `song.transport_loop` | `Option<LoopRegion>` | `SharedSong` | GUI, MCP `set_transport_loop` | Dirty: `global` fingerprint (not `song`, despite living in the `Song`). Undo: **none** | | Transport state persisted in the document | | Investigating |
| STATE-0046 | `song.next_*_id` (6 counters) + `pattern.next_note_id` | integers | `SharedSong` | Song mutation | Dirty: `song`, incidentally — the counters move whenever the collections do. Undo: **none**, and they never decrease, so an undo/redo cycle leaves them permanently advanced | | Allocator cursors persisted as document content — see `IDN-0024`, `IDN-0025` | | Investigating |

### State outside the project document

| ID | Field/state | Domain type | Current owner | Mirrors/save sources | Dirty/undo behavior | Intended V2 owner | Migration | Evidence | Status |
|----|-------------|-------------|---------------|----------------------|---------------------|-------------------|-----------|----------|--------|
| STATE-0047 | Sample library | `SharedSampleLibrary` | Session | Persisted **only** in a `.ptz.zip` bundle; a plain `.ptz` saves no samples although modules reference `sample_id` | Dirty: `samples`. Undo: `SetSample`, `SetSampleMeta`, `SetSampleData`, bounded by a 256 MiB retained-audio budget (`LIMIT-0064`) | | See `IDN-0010`, `IDN-0030`; asset identity is ADR-0017 | | Investigating |
| STATE-0048 | `AppSettings.theme`, `.font` | `ThemePreset`, `Option<String>` | `settings.json` (platform config dir) | GUI | Dirty: never — not project state. Undo: N/A | User settings | GUI-feature-gated fields | | Investigating |
| STATE-0049 | `AppSettings.author` | `Author` | `settings.json` | **Seed only.** Cloned into `current_project_author` at startup (`egui_backend.rs:608`) and used as the fallback when a loaded project carries no author. It is *not* the source of `ProjectFile.author` at save — see STATE-0003, which the first two passes both got wrong. | Dirty: never — editing it does not change the open project | User settings | | | Investigating |
| STATE-0050 | `AppSettings.directories` | 5 optional paths (patches, projects, last open/save/project dir) | `settings.json` | GUI file dialogs | Dirty: never. Undo: N/A | User settings | | | Investigating |
| STATE-0051 | `AppSettings.window` | width, height, x, y | `settings.json` | GUI | Dirty: never. Undo: N/A | User settings | | | Investigating |
| STATE-0052 | `AppSettings.recent_projects` | `Vec<PathBuf>`, max 10 (`LIMIT-0054`) | `settings.json` | GUI | Dirty: never. Undo: N/A | User settings | | | Investigating |
| STATE-0053 | `AppSettings.load_warning` | `Option<String>`, `#[serde(skip)]` | In-memory only | Set when settings fail to load | Dirty: never. Undo: N/A | Frontend-local transient | | | Investigating |
| STATE-0054 | Recovery snapshots | `RecoveryMeta { project_path, project_name, saved_at_unix }` + a full `ProjectFile`, max 20 entries (`LIMIT-0053`) | `RecoveryStore` | Autosave; `supersedes_manual_save` compares against the manual file's mtime | Dirty: **consumes** it — autosave compares the whole `ProjectRevision` and skips an unchanged snapshot. Undo: N/A | Runtime session | Recovery contract is Phase 10C | | Investigating |
| STATE-0055 | User group templates | `GroupTemplate` JSON files | `GroupTemplateManager` | Read from disk alongside 12 built-ins | Dirty: never — not project state. Undo: N/A | User settings | | | Investigating |
| STATE-0056 | Bundle metadata | `BundleMetadata`, `BundleSampleEntry`, `BundleLoopRegion`, `BundleCropRegion` | `.ptz.zip` | Bundle save | Dirty: via `samples`. Undo: N/A — regenerated from the library at save | Project document | The bundle root has one property, `samples`; each entry carries id, name, channels, frame_count, sample_rate, root_note, crop, loop_region | | Investigating |
| STATE-0060 | MCP project-author slot | `Arc<Mutex<Option<Author>>>` | `McpSharedState.author` (`mcp_shared.rs:115`) | A second live copy of STATE-0003, mirrored from the project on load and read by the MCP save path's own `ProjectBuildOptions` (`mcp_bridge.rs:1131-1145`) | Dirty: not observed by any term | Runtime session | Must collapse into one owner together with STATE-0003 | | Investigating |

### Session state discovered by the workflow trace

| ID | Field/state | Domain type | Current owner | Mirrors/save sources | Dirty/undo behavior | Intended V2 owner | Migration | Evidence | Status |
|----|-------------|-------------|---------------|----------------------|---------------------|-------------------|-----------|----------|--------|
| STATE-0057 | Autosave scheduling state | `AutosaveState` — store handle, last attempt, last snapshotted `ProjectRevision`, `snapshotted_path`, failure-reported flag, in-flight write | GUI app shell | Not persisted | Is itself part of the dirty mechanism | Runtime session | `snapshotted_path` exists because Save As and the `.ptz`→`.ptzb` normalization move the current path out from under a snapshot keyed by the old one | | Investigating |
| STATE-0058 | Saved-state baseline | `saved_revision: ProjectRevision`, `saved_undo_position`, `untracked_mutation_since_save` | GUI app shell | Not persisted | This *is* the dirty baseline | Runtime session | `untracked_mutation_since_save` latches for the rest of the session once set | | Investigating |
| STATE-0059 | Undo/redo stacks | `Vec<UndoAction>` — 60 variants, depth 100 (`LIMIT-0063`) | `UndoManager` | Not persisted; discarded on load | Undo history does not survive save/load, so "clean" after reopening a project is decided by the counters alone | Runtime session | History representation is ADR-0015 | | Investigating |

## Required workflow coverage

Audit GUI save, MCP save, autosave, recovery, rollback, patch/preset save, bundle save, import/export, and offline
render reconstruction. A field is not complete until every current mirror and save source is understood.

### Save-path map

All project save paths converge on one builder with one command-drain barrier:

- `build_project_from_engine` (`crates/pertylizer/src/project_apply.rs:583`) reconstructs the project from engine +
  session + song state, after `session.wait_for_pending_commands(SNAPSHOT_SYNC_TIMEOUT_MS)`. It writes
  `Position::default()` for every module and no group/canvas metadata.
- The **GUI overlay** (`project_flow.rs:561`, `overlay_ui_metadata`) supplies module positions, groups, canvas size,
  and visualizer modules from `PatchEditor` state, and writes `active_instrument_id` from the GUI's own selection.
- **MCP save and rollback** (`build_project_for_persistence`, `crates/pertylizer/src/mcp_bridge.rs:948`) request the
  GUI's project first and fall back to the engine reconstruction when no GUI is attached *or* when the GUI does not
  answer within `GUI_PROJECT_SNAPSHOT_TIMEOUT_MS`. The timeout case logs a warning and silently loses the overlay.

The two paths do **not** share their `ProjectBuildOptions` sources: the GUI fills them from `SynthApp` state
(`project_flow.rs:568`) and MCP from `McpSharedState` (`mcp_bridge.rs:1131`). STATE-0003/STATE-0060 is the first
confirmed case where those two sources hold the same concept separately.

### Workflows traced

- **Autosave** (`gui/egui_backend/autosave_flow.rs`) — debounced at 30 s (`LIMIT-0066`), skipped when the
  `ProjectRevision` is unchanged since the last snapshot, and written on a worker thread because a sample-heavy project
  serializes to a ZIP with every sample embedded. It builds through `create_project_from_app()`, i.e. **the GUI path**,
  so a recovery snapshot carries the layout overlay. It never writes to the user's project file. At most one write is
  in flight.
- **Patch save** (`project_apply.rs:492`) — has its own two-part barrier rather than the shared one: a command drain
  *and* a bounded wait for the instrument to appear in the async mirror, so an instrument created in the same MCP
  batch is not saved stale or truncated.
- **Bundle save/load** (`bundle.rs`) — save writes the project JSON plus one WAV per sample, keyed by `SampleId`.
  **Load calls `library.clear()` first**, so opening a bundle discards whatever samples the session held; ids are
  restored verbatim with `add_with_id` and never remapped. There is no merge or import-into-session path.

### Workflows still not traced

- **Rollback** as a distinct path — it shares `build_project_for_persistence` with MCP save, but the restore side was
  not read.
- **Preset/example-patch save**, and the 68 programmatic built-in patches as a *write* source.
- **Import/export** — WAV export and sample import. Tracker-module import is on an unmerged branch and is not present
  at `dd69b657`.
- **Offline render reconstruction** as a *reader* of this state, including whether it sees the overlay fields at all
  (it does not: the render CLI has no GUI, so it takes the engine-reconstruction fallback by design).

## Audit passes

| Date | Source revision | Paths inspected | Coverage/result | Evidence |
|------|-----------------|-----------------|-----------------|----------|
| 2026-08-12 | `dd69b657` | `schemas/project.schema.json` walked programmatically (121 `$defs`, 276 declared properties); `crates/pertylizer/src/project.rs`, `io/settings.rs`, `recovery.rs`, `bundle.rs` read directly; save paths read in `project_apply.rs` and `mcp_bridge.rs`. | 56 entries at section granularity. Three duplicate-value conflicts recorded (later corrected), one split concept (STATE-0010/STATE-0043), one shape mismatch (STATE-0036), and the one documented GUI overlay. The `Dirty/undo behavior` column is entirely unpopulated. | Pending `EVD` record for P00B-T001 |
| 2026-08-12 | `dd69b657` | `crates/pertylizer/src/dirty.rs` (all seven revision terms), `undo.rs` (60 `UndoAction` variants enumerated by a brace-depth parser), `gui/egui_backend.rs:754-780`, `gui/egui_backend/undo_flow.rs`, `gui/egui_backend/autosave_flow.rs`, `project_apply.rs:492`, `bundle.rs:175-280`. | `Dirty/undo behavior` filled for all entries; 3 session entries added (STATE-0057..0059); three workflows traced. Findings: **`active_instrument_id` (STATE-0004) is observed by no dirty term** — the three instrument-switch sites set it without `mark_dirty()` while every save writes it, so changing the focused instrument changes the saved file with no `*`, no autosave snapshot and no close prompt. The four `PatchSettings` value fields turned out **not** to be live duplicates (STATE-0034/0035): the project save path writes them as constants, correcting pass 1's "persisted twice, precedence unknown" reading. 14 entries have no undo action. Method limit: undo coverage is read from the variant list and dispatch arms, not exercised. | Pending `EVD` record for P00B-T001 |
| 2026-08-12 | `dd69b657` | Review follow-up: `gui/egui_backend.rs:552,608,4367-4397`, `gui/egui_backend/project_flow.rs:560-573,758-760`, `mcp_shared.rs:115`, `mcp_bridge.rs:1131-1145`. | Corrected STATE-0003 and STATE-0049, added STATE-0060. Both earlier passes recorded `ProjectFile.author` as coming from `AppSettings.author`; it does not — the save path reads a separate per-project field, the settings value is only a seed for a new project, and MCP holds a third copy. **The error came from reading the declaration (`ProjectBuildOptions.author`) rather than following who fills it** — precisely the mistake this ledger exists to catch, so every remaining `Mirrors/save sources` cell derived the same way should be treated as unconfirmed until a caller has been traced. All `Classified` statuses were also downgraded: the register vocabulary requires supporting evidence and the `Evidence` column is empty throughout. | Pending `EVD` record for P00B-T001 |
