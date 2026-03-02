# TODO - Modular Synth (v0.200.0)

## Priority 1 — Foundation & Core Functionality

### 1.1 Settings expansion
- [x] Remember last used directory when loading/saving patches
- [x] Show all directory paths in Settings dialog (patches, settings file, etc.) with ability to change them
- [x] Show group templates directory in Settings dialog
- [x] Add explicit Save button in Settings dialog
- [x] Show the settings file path in Settings dialog
- [x] Auto-save last used directory to settings when loading or saving a patch
- [ ] Add Browse button in Settings dialog to change patches directory (needs `rfd` dependency or reuse `egui-file-dialog` with directory picker)

### 1.2 Song save/load (full project persistence)
- [x] **Step 1: Define song file format** — JSON `ProjectFile` with `file_type: "project"`, instruments, song, global state
- [x] **Step 2: Song serialization** — `create_project_from_app()` serializes all instruments + song + global state
- [x] **Step 3: Song deserialization & load** — `load_project_data()` clears state and recreates from project file
- [x] **Step 4: Save/Load UI** — File menu with Open Project, Save Project, Save Project As; auto-detect patch vs project
- [ ] **Step 5: Recent projects** — Remember last opened projects in settings, show in menu for quick access
- [ ] **Step 6: Dirty state tracking** — Warn on unsaved changes before loading or quitting

### 1.3 Undo/Redo
- [ ] Implement undo/redo for module operations (add, delete, move, parameter changes)
- [ ] Implement undo/redo for connection operations (add, remove)
- [ ] Keyboard shortcuts: Ctrl+Z / Ctrl+Shift+Z

### 1.4 Copy/paste modules
- [ ] Copy a module with its current parameters
- [ ] Paste as a new instance with the same settings
- [ ] Consider: copy a selection of modules + their internal connections

### 1.5 Audio export
- [ ] Render arrangement to WAV file (offline, faster-than-realtime)
- [ ] Export dialog: file path, sample rate, bit depth, duration/range
- [ ] Progress bar during render

### 1.6 Template library (groups + patches) — see `docs/Template-Library-Plan.md`
- [x] Show all group templates from the template directory in one list (no curated/user split)
- [x] Add `Save as Template` in Group Modules menu (write group template JSON with metadata)
- [ ] Add patch template directory and `Save Patch as Template` action
- [ ] Add Patch Template browser (or File menu section) to load patch templates
- [ ] Support optional `license` and `min_app_version` metadata in group templates

---

## Priority 2 — UI Structure & Layout

### 2.1 Effects section — visual separation from voice modules
- [ ] Add a visual divider between voice modules and effects in the grid
- [ ] Use a distinct background color/tint for the effects area (right side)
- [ ] Alternative: Separate "Effects" panel below or beside voice grid, with its own header
- [ ] Label the section clearly: "Master Effects" or "Effect Chain"
- Current state: Effects show as "Effect Chain" with full-width layout, but distinction is subtle.

### 2.2 Cable rendering — clip to patch area
- [x] Ensure cables don't render outside the patch/rack view bounds when hovered
- Fixed: Active/hovered cables now use `Painter::new()` with explicit clip rect instead of `layer_painter()` which defaulted to `Rect::EVERYTHING`

### 2.3 Module Groups — Phases 1–3 (see `docs/Module-Groups-Plan.md`)
- [x] Phase 1: Visual grouping data model + patch serialization (`groups`)
- [x] Phase 1: UI for create/expand/collapse, expose ports, drag in/out
- [x] Phase 1: Enforce exclusivity (module belongs to one group)
- [x] Phase 1: Enforce boundary rule (all external connections via exposed ports, always)
- [x] Phase 1: Group operations — delete group (delete contents) and ungroup (keep contents)
- [x] Phase 1: Rename groups + group color picker
- [x] Phase 1: Group port rendering (hover + connected state)
- [x] Phase 1: Collapsed group ports mirror module orientation + expand/collapse icon
- [x] Phase 2: Group templates (format, storage, browser)
- [x] Phase 2: Template instantiation with full ID remapping + drop-point layout
- [x] Phase 2: Group template browser supports file picker (Browse...)
- [ ] Phase 2: Template variants (parameter presets with remap)
- [ ] Phase 3: Probes data pipeline (ringbuffers, audio-thread safe collection)
- [ ] Phase 3: Probe rendering (waveform/spectrum/meter) with PortType-based signal type
- [ ] Phase 3: Polyphony probes = sum of voices (mixdown)

---

## Priority 3 — Visual Polish

### 3.1 Improve module knobs
- [ ] Better visual design — consider: gradient fill, shadow, tick marks, value tooltip
- [ ] Consistent sizing across module types
- [ ] Consider: Arc-style knobs with colored fill showing current value

### 3.2 Improve module ports
- [ ] Clearer port type distinction (audio vs control vs gate vs MIDI)
- [ ] Better hover feedback
- [ ] Consider: Colored rings matching cable colors, port labels on hover

---

## Priority 4 — AWE Improvements

### 4.1 Rework room visualization
- [ ] Redesign the 3D isometric room rendering
- [ ] Improve animations (sound rings, reflection paths)
- [ ] Better visual clarity for room shape and dimensions

### 4.2 Differentiate effects more clearly
- [ ] Each material/effect should have more distinct visual representation
- [ ] Consider: Color-coded zones, animated textures per material, spectral visualization
- [ ] Alternative: Show frequency response curves for each material type

---

## Priority 5 — Future / Later

### 5.1 Redesign instrument list
- [ ] Current: Scrollable list with inline controls (name, MIDI channel, volume, pan, mute/solo)
- [ ] Consider: Tabbed interface, mixer-style vertical strips, or collapsible panels

### 5.2 MIDI learn
- [ ] Map MIDI CC to any module parameter via right-click → "MIDI Learn"
- [ ] Visual indicator on mapped parameters
- [ ] Save/load MIDI mappings with patch or settings

### 5.3 Module presets
- [ ] Save/load parameter presets per module type (not the whole patch)
- [ ] Preset browser in module context menu or header
- [ ] Ship default presets for common module types (filters, envelopes, etc.)

### 5.4 Polyphony settings
- [ ] Voice count configurable per instrument (GUI control)
- [ ] Voice stealing mode selection (oldest, quietest, none)
- [ ] Unison detune/spread controls

---

## Completed

<details>
<summary>Click to expand completed items</summary>

### Enable MCP feature by default (v0.188.0)
- [x] Change `default = ["gui-egui"]` to `default = ["gui-egui", "mcp"]`

### English language everywhere (v0.165.0)
- [x] Translate all Swedish UI strings to English

### Settings file — persistent config (v0.170.0)
- [x] Create settings file, load/save, theme, window position, author info

### Remove debug module (v0.160.0)
- [x] Remove `debug/` directory, feature flag, examples, CLAUDE.md entry

### Sequencer — Track & pattern management from GUI (v0.186.0)
- [x] Add/remove/rename tracks and patterns, edit pattern length, assign instrument, mute/solo, loop toggle

### Remove module toolbar + improve right-click context menu (v0.180.0)
- [x] Toolbar removed, context menu reworked with proper submenus, Swedish strings translated

### Keyboard area cleanup (v0.182.0)
- [x] Gap fixed, PANIC moved, label removed, octave controls inlined

### Tab buttons — segmented control (v0.187.0)
- [x] Pill-shaped connected buttons with filled active tab

### MCP connection status indicator (v0.188.0)
- [x] Robot icon with green/dim/red states, hover tooltip, session tracking

### Auto-layout after patch load (v0.190.0)
- [x] MCP reconciliation triggers auto-layout; file/example loads use saved positions

### Save module positions in patch JSON
- [x] `ModuleState.position` serialized, saved, and restored on load

### Switch to egui-remixicon for all icons (v0.185.0)
- [x] All GUI icons replaced; only `⚠` remains in console.rs (text-only)

### Improve module header icons (v0.190.0)
- [x] Meaningful icons, always-visible delete, cable bypass on delete

### Reduce module-type registration boilerplate (v0.190.0)
- [x] Removed `PatchModuleType`, `ModuleState` uses `synth_core::ModuleType` directly

### MCP auto layout (v0.193.0)
- [x] New `auto_layout` MCP tool triggers GUI auto-layout via shared AtomicBool

### Group template browser & save (v0.193.0)
- [x] Save group as template (JSON with metadata)
- [x] Template browser dialog with search and insert

</details>
