# TODO - Modular Synth (v0.189.0)

## Priority 1 — Foundation & Core Functionality

### 1.1 Enable MCP feature by default
- [x] Change `default = ["gui-egui"]` to `default = ["gui-egui", "mcp"]` in `crates/modular_synth/Cargo.toml`

### 1.2 English language everywhere
- [x] Translate all Swedish UI strings to English
- Translated: awe_view.rs (materials, labels, tooltips), patch_editor.rs (context menus)
- Code comments in theme.rs remain Swedish (not user-facing)

### 1.3 Settings file (persistent config)
- [x] Create settings file (`~/.local/share/modular-synth/settings.json`)
- [x] Load on startup, save on change and on exit
- [x] Theme persisted and restored at startup
- [x] Window position and size persisted
- [x] Author info in Settings dialog: name, email, website, license
- Later expansion: MIDI device preferences, default BPM, audio buffer size, etc.

### 1.4 Remove debug module — superseded by MCP
- [x] Remove `crates/modular_synth/src/debug/` (4 tools, ~2100 lines)
- [x] Remove `debug-tools` feature from `Cargo.toml`
- [x] Remove `#[cfg(feature = "debug-tools")] pub mod debug;` from `lib.rs`
- [x] Remove example programs: `examples/debug_graph.rs`, `examples/midi_test.rs`
- [x] Remove `debug_graph` entry from CLAUDE.md debug tools table

### 1.5 Sequencer — Track & pattern management from GUI
- [x] Add/remove/rename tracks from GUI (currently MCP-only)
- [x] Add/remove/rename patterns from GUI
- [x] Edit pattern length from GUI
- [ ] Edit track length from GUI
- [x] Assign instrument to track from GUI
- [x] Show which instrument is selected/active in piano roll (header or color coding)
- [x] Add mute/solo buttons per track in track header
- [x] Add repeat/loop toggle checkbox for playback
- Sequencer is now self-contained in the GUI (v0.186.0)

---

## Priority 2 — UI Structure & Layout

### 2.1 Remove module toolbar + improve right-click context menu
- [x] **Remove the "Add module" toolbar** — the `TopBottomPanel::top("toolbar")` row below the menu bar. Move the Glide slider and module/connection counts into the menu bar (right-aligned section, Rack view only).
- [x] **Rework the right-click context menu** in `patch_editor.rs`:
  - Replace the current frameless buttons (Filter, Envelope, LFO, VCA, Mixer) with proper `ui.menu_button()` submenus or standard menu items with consistent styling
  - All module categories should use `ui.menu_button()` with submenus when there are multiple choices, or a single `ui.button()` menu item when there's only one option (e.g. Output)
  - Cable actions (Delete cable, Insert Signal Monitor) should also be proper menu items
  - Translate remaining Swedish strings to English ("Ta bort sladd", "Stoppa in Signal Monitor")
- [x] **Keep right-click as the primary way to add modules** — no "Add" menu in top menu bar needed

### 2.2 Keyboard area cleanup
- [x] Fix gap between piano keys and visualizers (persistent spacing issue)
- [x] Move PANIC button to top menu bar (right side, keep red styling)
- [x] Remove "KEYBOARD" label — the piano keys are self-explanatory
- [x] Clarify "Playing: Default" — this shows active instrument name. Rename to "Active: {name}" or show instrument icon + name
- [x] Move octave +/- controls inline with the keyboard or into a compact row to eliminate the full-width control row

### 2.3 Tab buttons — make Seq/AWE/Rack clearer
- [x] Use larger, styled tab buttons with icons (once egui-remixicon is available)
- [x] Consider: icons + text, active tab underline/highlight, or segmented control style
- Implemented as segmented control: pill-shaped connected buttons with filled active tab (v0.187.0)

### 2.4 MCP connection status indicator
- [x] Show MCP status icon next to MIDI status in top bar
- [x] States: connected (green dot), disconnected (gray dot), error (red dot)
- Implemented with `AtomicBool` (listening) and `Arc<AtomicUsize>` (active sessions) in `McpSharedState` (v0.188.0)
- Robot icon: filled green when sessions active, dim outline when listening, red when not running
- Hover tooltip shows detailed status

### 2.5 Effects section — visual separation from voice modules
- [ ] Add a visual divider between voice modules and effects in the grid
- [ ] Use a distinct background color/tint for the effects area (right side)
- [ ] Alternative: Separate "Effects" panel below or beside voice grid, with its own header
- [ ] Label the section clearly: "Master Effects" or "Effect Chain"
- Current state: Effects show as "Effect Chain" with full-width layout, but distinction is subtle.

### 2.6 Cable rendering — clip to patch area
- [ ] Ensure cables don't render outside the patch/rack view bounds when hovered
- Current: `painter.with_clip_rect(inner_rect)` exists but may not account for hover glow layers
- Check: The glow effect (8px outer, 5px inner) may extend beyond clip rect

### 2.7 Auto-layout after patch load
- [ ] Run auto-layout when a patch is loaded from file or selected from examples
- [ ] Ensure module positions settle before user interaction
- Related: Save/restore module positions (see 3.1)

---

## Priority 3 — Visual Polish

### 3.1 Save module positions in patch JSON
- [ ] Add `position: (x, y)` to `ModuleState` in patch serialization
- [ ] Save positions when writing patch to disk
- [ ] Restore positions when loading (fall back to auto-layout if missing)
- Check: MCP patch format compatibility — ensure `PatchBridge` and file I/O share serialization.
  Current patch format uses `serde_json` with `Patch { modules, connections, settings }`.
  MCP builds patches via `session.add_module_with_id()` — positions are GUI-only state.

### 3.2 Switch to egui-remixicon for all icons
- [x] Add `egui-remixicon` dependency
- [x] **Test phase:** Replace one icon set (e.g., module category icons) and verify rendering
- [x] Replace module header icons (source/sink indicators, connectivity, power/bypass, close)
- [x] Replace File menu icons (New, Open, Save, Settings, Quit, Example Patches)
- [x] Replace context menu icons (all palette items, submenu headers)
- [x] Replace MIDI dropdown icons
- [x] Replace instrument rack radio buttons
- [x] Replace transport controls (play/stop/seek/pause)
- [x] Replace tab icons (Seq/AWE/Rack)
- [x] Replace all remaining emoji/Unicode symbols
- Only remaining emoji: `⚠` in console.rs (text-only output, not GUI).

### 3.3 Improve module knobs
- [ ] Better visual design — consider: gradient fill, shadow, tick marks, value tooltip
- [ ] Consistent sizing across module types
- [ ] Consider: Arc-style knobs with colored fill showing current value

### 3.4 Improve module ports
- [ ] Clearer port type distinction (audio vs control vs gate vs MIDI)
- [ ] Better hover feedback
- [ ] Consider: Colored rings matching cable colors, port labels on hover

### 3.5 Improve module header icons
- [ ] Replace Unicode symbols with proper icons (egui-remixicon, depends on 3.2)
- [ ] Better visual hierarchy — power button more prominent, status icons more subtle

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
- [ ] TBD: Gather more specific requirements

### 5.2 Patch/MCP serialization unification
- [ ] Audit: Compare file-based patch format with MCP's `build_instrument` / `apply_example_patch` format
- [ ] Goal: Single serialization path for save/load/MCP, reducing duplication
- [ ] `patch_bridge.rs` already bridges between formats — evaluate if it can be the single source of truth
