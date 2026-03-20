# Version History

## [0.248.0] - 2026-03-20
### AWE: Eyring RT60 and extended room modes
- **Replaced Sabine RT60 with Eyring formula** — `RT60 = -0.161V/(S·ln(1-α))` gives more accurate decay times at high absorption (e.g. carpet, fabric rooms)
- **Extended room modes from 3 to 12** — added axial overtones (2,0,0)/(0,2,0)/(0,0,2), tangential modes (1,1,0)/(1,0,1)/(0,1,1)/(2,1,0)/(1,2,0), and oblique mode (1,1,1) using general formula `f = c/2·√((nx/L)²+(ny/W)²+(nz/H)²)`
- **Gain-weighted mode bank** — tangential modes at 0.7×, higher tangential at 0.5×, oblique at 0.4× relative to axial fundamentals
- **Fixed newtype violation** — `MODE_GAINS` and `CombFilter::update` now use `Gain` instead of raw `f32`
- **Fixed pre-existing clippy warning** — `field_reassign_with_default` in test replaced with struct update syntax

## [0.247.0] - 2026-03-18
### AWE fixes: RT-safety, div-by-zero guards, portal scaling, Swedish→English
- **Translated all Swedish text to English** — doc comments in room.rs (10 material descriptions), preset descriptions in presets.rs, module docs in awe_engine.rs and spatial_voice.rs
- **Fixed portal delay not scaling by room size** — was a fixed 200ms despite comment claiming room-size scaling; now uses avg dimension ratio
- **Fixed division-by-zero in axial_modes()** — dimensions clamped to minimum 0.01 before dividing
- **Fixed room_modes bypass state inconsistency** — bypass path now calls mode.process() to maintain comb filter state during amount=0
- **Fixed unbounded early reflection gain** — clamped per-tap gain to 2.0 (was up to 10x at MIN_DISTANCE)
- **Fixed RT-safety in spatial_voice** — update_slot/process_slot/info/buffer use bounds-checked indexing instead of panicking
- **Fixed Position3 Index/IndexMut panic** — index clamped to 0..=2
- **Fixed small room position mapping** — LinearX/LinearY guard against rooms < 0.6m
- **Fixed spatializer LP coefficient comment** — was inverted ("closer to 0" vs actual "closer to 1")
- **Extracted FdnParams helper** — eliminated large code duplication between process() and process_spatial()
- **Replaced magic numbers with named constants** — AVG_REFERENCE_DIMENSION, REFERENCE_SAMPLE_RATE, AVG_BASE_FDN_DELAY, DEFAULT_PORTAL_FEEDBACK/DAMPING
- **Fixed stale doc comment** — early reflections default was "14400" but actual is 96000
- **Removed redundant NormalizedValue re-wrapping** in early_reflections
- **Precomputed INV_NUM_MODES** — replaced division with multiplication in audio path
- **Fixed no-op test assertion** — lfo.rs `|| true` replaced with proper range check

## [0.246.0] - 2026-03-18
### Sequencer code quality: #[must_use] enforcement and count validation
- **Removed crate-level `#![allow(clippy::must_use_candidate)]`** — added explicit `#[must_use]` to all public newtypes, constructors, builder methods, and query methods across the synth_sequencer crate
- **Fixed TrackCount/RowCount allowing 0** — constructors and `From` impls now clamp to minimum 1; updated docs to reflect valid range
- **Fixed unused return values in tests** — pattern test code now properly handles `NoteId` returns from `add_note`/`insert_note`
- **Fixed `vec!` lint in event sorting test** — replaced with `Vec::from` array construction

## [0.245.0] - 2026-03-18
### Application fixes: RT-safety, error handling, UX improvements
- **Fixed RT-safety in CPAL error callback** — removed blocking Mutex lock and heap-allocating format! from audio thread error callback; uses atomic counter only
- **Fixed export thread spawn failure** — spawn errors now set progress error and mark completed instead of silently returning hanging progress
- **Fixed MCP parameter value return** — `set_parameter` was returning parameter ID instead of actual value
- **Fixed session clear_graph desync** — command send failures now checked; registry preserved for modules whose removal failed to send
- **Fixed Tokio panic** — MCP server startup gracefully degrades instead of panicking on runtime creation failure
- **Fixed session counter reset** — `apply_patch()` now resets module ID counters before loading, preventing ID mismatches
- **Fixed auto-layout dirty flag** — project now marked as modified after auto-layout changes module positions
- **Fixed MCP add_notes batch** — per-note error handling with pitch validation, matching update_notes pattern
- **Fixed title bar allocation** — cached last title string, only updates viewport when title actually changes
- **Export instrument warnings** — failed instrument loads during export now reported as warnings to the GUI
- **Preview duration validation** — rejects zero-duration preview requests with descriptive error
- **MIDI error logging** — dropped MIDI messages and connection failures now logged via eprintln
- **Patch load parse warnings** — invalid module IDs during patch load now logged for diagnostics
- **Settings load warning** — added `load_warning` field shown as GUI toast when settings fail to load
- **Patch directory error logging** — `flatten()` replaced with explicit error handling for directory entries
- **NullStream documented** — single-use limitation documented on start() method
- **Undo stubs documented** — AddModule/RemoveModule undo now logs warning instead of silent no-op
- **Envelope label fix** — sustain point label offset adjusted to avoid overlap with release label
- **Knob bounds check** — double-click default reset now clamped to valid range
- **Meter clip threshold** — changed from >0.98 to >=1.0 for accurate 0dB clip detection
- **Tooltip refactored** — extracted shared rendering logic into `render_tooltip()` helper
- **Spectrum label overlap** — frequency labels now skip drawing when they would overlap previous label

## [0.244.0] - 2026-03-17
### DSP module fixes: RT-safety, numerical stability, and logic bugs
- **Fixed RT-safety violations** — phase_vocoder, spectral_blur, signal_monitor, compressor, granular_osc, euclidean: eliminated heap allocations in audio thread by pre-allocating buffers and using fixed-size arrays
- **Fixed oscillator unison panning** — pan angle calculation used π/4 instead of π/2, giving only half the stereo width
- **Fixed math oscillator stability** — clamped logistic map to [0,1] preventing NaN divergence; guarded Karplus-Strong against zero-frequency division
- **Fixed MSEG release transition** — releasing gate before sustain now correctly skips to post-sustain segment instead of replaying current segment
- **Fixed LFO retrigger edge detection** — prev_retrigger state now always tracks input regardless of retrigger mode, preventing stale state on mode toggle
- **Fixed LFO RT-safety** — replaced `PortName::intern("retrigger")` (takes RwLock) with compile-time `PortName::RETRIGGER` constant
- **Fixed compressor silent sidechain** — clamped detect_peak to 1e-6 before dB conversion, preventing -infinity envelope
- **Numerical stability** — fractal_osc sum_a guard strengthened to 1e-7; EQ biquad a0 zero-guard; delay feedback decay clamped to 0.99; keyboard_panner curve singularity clamped to ±0.99
- **Debug assertions** — ensemble_chorus and shimmer_reverb now assert sample rate consistency in process()

## [0.243.0] - 2026-03-17
### Engine real-time safety fixes and MCP input validation
- **Fixed Vec allocation in audio thread** — `graph.rs` process_module() no longer heap-allocates per frame; added `InputPorts::from_owned()` to accept owned buffers directly
- **Fixed recording heap allocations** — `flush()` and `take_released_notes()` now use swap-based approach with pre-allocated spare Vec instead of `Vec::with_capacity()` on audio thread
- **Fixed spatial voice bank sample count** — oversampled path now decimates audio before writing to spatial bank instead of passing wrong sample count
- **Fixed voice stealing** — `steal_for()` stores pending note that activates after fade-out completes, instead of immediately overwriting the Stealing state
- **Fixed shared_state race conditions** — `is_live()` TOCTOU race eliminated; `bump_version()` now called inside write lock scope in 9 methods
- **Fixed focused instrument routing** — uses `InstrumentId` directly instead of stale vector index that could become invalid after instrument add/remove
- **Consistent release detection** — both oversampled and normal voice paths now use peak-based silence detection
- **Eliminated panic in EngineCommand clone** — replaced `panic!` with `try_clone() -> Option<Self>` for non-clonable variants
- **Pre-allocated audio buffers** — effect chain working buffer and voice allocator held_notes pre-allocated to prevent runtime allocation
- **Click generator safety** — early return if sample_rate is zero
- **MCP input validation** — 34+ tool handlers now validate all inputs (MIDI note/velocity/channel ranges, volume/pan/tempo bounds, pattern lengths, automation values, time signatures, array indices, non-empty names) with descriptive error messages for AI self-correction
- **MCP error improvements** — 8 new typed error variants replace generic `Other(String)`; `CommandSendFailed` now includes command name context
- **Pan range unified** — MCP API consistently uses bipolar -1.0..1.0 for both instrument and track pan with conversion at bridge boundary
- **Serialization helper** — extracted `to_json()` replacing 38 repeated serialization blocks

## [0.242.0] - 2026-03-17
### Visualizer: Bevy events, material buckets, and code quality fixes
- **Bevy Message events** — replaced manual `pending_note_events: Vec` buffer in `SynthTelemetry` with Bevy's `Message` system (`NoteOnEvent`, `CameraModeEvent`); OSC receiver writes via `MessageWriter`, effects consume via `MessageReader`
- **Material buckets for all effects** — centroid_nebula (16 spatial buckets), ferrofluid_tendrils (32 frequency-mapped), pulse_terrain (16 distance-based with center-bright gradient), spectral_origami (32 hue-varied), voronoi_shatter (32 with saturation/lightness variation), chord_bloom (alternating segment colors), note_tree (3 level-based: trunk/branch/twig)
- **FFT terrain Z-fade** — per-row lightness and emissive fade backward in Z for depth
- **Crit mechanics** — 5% chance for instrument_cubes and velocity_meteors to spawn as "crit" (white, 2x size, 3x spin)
- **Harmonic ribbons pitch bend** — Z-scale pulses drastically with pitch_bend
- **Meteor rotation** — meteors now spin as they fall, crit meteors spin faster
- **Fixed compilation errors** — removed non-functional `shader_test` module that broke build (invalid `ShaderRef` import, misplaced `mod` before doc comment)
- **Eliminated duplicated logic** — extracted `bucket_color()` helper in pulse_terrain, refactored voronoi_shatter and spectral_origami to use `create_hue_materials()` consistently

## [0.241.0] - 2026-03-14
### Fix modules invisible after project load
- **Removed `reset_areas()` call** — `ctx.memory_mut(|mem| mem.reset_areas())` was clearing egui's internal Area state on the frame after project load, making module panels invisible until the user switched instruments and back; since `current_pos()` already overrides stored positions every frame, the reset was unnecessary
- **Cleaned up debug code** — removed frame-by-frame position logging, reconcile file logging, and load diagnostics that were added during the position bug investigation

## [0.240.0] - 2026-03-14
### Fix module positions lost on project load
- **Root cause: MCP reconciliation race** — `reconcile_instruments()` compared GUI state against engine's shared state, but engine processes `AddInstrument` commands asynchronously; during the delay, reconciliation removed just-loaded GUI instruments and recreated them with empty PatchEditors, losing all saved module positions
- **Fix: preserve instruments with modules** — `reconcile_instruments` no longer removes GUI instruments whose PatchEditor contains modules, preventing loaded state from being destroyed during async engine sync
- **Suppress stale position readback** — `PatchEditor` skips reading positions back from egui Area state during the frame a patch is loaded, preventing stale cached positions from overwriting saved ones
- **Clear active instrument on load** — `active_instrument_id` is reset to `None` when loading a project or creating a new one

## [0.239.0] - 2026-03-14
### Engine starts with no instruments
- **No default instrument** — engine no longer creates `InstrumentId::FIRST` at startup; all instruments are created explicitly via `AddInstrument` command
- **`active_instrument_id` is `Option<InstrumentId>`** — GUI properly represents "no instrument selected" state instead of assuming one always exists
- **Removed `SynthEngine::with_config()`** — allocator config was only used for the default instrument; engine constructor is now just `SynthEngine::new()`
- **Removed FIRST special-casing** — project loading and export no longer skip `InstrumentId::FIRST` when creating instruments, fixing module position bugs for multi-instrument projects
- **MCP sync cleanup** — removed safety retain that prevented removing the default instrument

## [0.238.0] - 2026-03-14
### Sorted JSON keys
- **BTreeMap for serialization** — replaced `HashMap` with `BTreeMap` in all serialized structs (`ModuleState.parameters`, `Song.patterns`, `Song.tracks`) so JSON keys are always written in alphabetical order
- **Ord derives** — added `PartialOrd`/`Ord` to `PatternId` and `TrackId` to support `BTreeMap` usage

## [0.237.0] - 2026-03-13
### Reorder effect chain
- **Effect chain reorder** — `move_slot_up()` / `move_slot_down()` methods on `EffectChain` to swap slot positions, with `ReorderEffect` engine command + `ReorderDirection` enum
- **Effect chain cables** — visual amber/orange cables drawn between effects in chain order with arrowheads showing signal flow direction, plus IN/OUT labels at chain endpoints
- **Chain position indicator** — effect modules show their chain position (#1, #2, ...) with up/down arrow buttons for quick reordering
- **Auto-layout respects chain order** — when auto-layout runs, effect modules are arranged top-to-bottom matching their processing order
- **`InstrumentSnapshot.effect_chain_order`** — shared state now exposes the ordered list of module IDs in the effect chain for GUI access

## [0.236.0] - 2026-03-13
### Canvas size serialization
- **`CanvasSize` struct** — replaced `Option<(f32, f32)>` tuple with a named `CanvasSize { width, height }` struct for canvas size in patch settings
- Serializes as `{"width": ..., "height": ...}` object instead of `[w, h]` array
- Backward-compatible deserialization: reads both new object format and legacy `[w, h]` arrays
- Updated all example patches and projects to use the new format

## [0.235.0] - 2026-03-13
### Clean up TODO
- Remove completed items from TODO.md, renumber remaining sections

## [0.234.0] - 2026-03-13
### MCP audio preview via RawAudioContent
- **`preview_note` MCP tool** — new tool that renders a note on any instrument offline and returns the audio as a base64-encoded WAV clip via rmcp's `RawAudioContent`, allowing AI agents to hear what a patch sounds like
- **Offline note preview renderer** — `audio::preview::render_note_preview()` snapshots an instrument's module graph and parameters from the live engine, creates a temporary offline engine, plays a note for a configurable duration with tail time, and produces 16-bit 44.1kHz WAV data in memory
- **`AudioPreview` bridge type** — new `render_note_preview` method on `SynthBridge` trait, returning WAV bytes with metadata (sample rate, duration)

## [0.233.0] - 2026-03-13
### Upgrade rmcp 0.17 → 1.2.0
- **Upgrade rmcp to 1.2.0** — bumped from 0.17 to 1.2, the first stable release of the MCP protocol library
- **Enhanced MCP server metadata** — `get_info()` now reports title ("Pertylizer"), description, and website URL via the new `Implementation` fields
- **Fix non-exhaustive struct construction** — migrated `ServerInfo` and `ReadResourceResult` to use builder/constructor patterns required by rmcp 1.x

## [0.232.0] - 2026-03-12
### Fix WAV export and project module layout restoration
- **Fix WAV export producing no file** — `FileDialogMode::ExportWav` was missing from the `Saved` pattern match in `update_file_dialog()`, causing the save path to be silently discarded
- **Fix module positions not restored on project load** — egui caches Area positions by ID; loading a project now calls `reset_areas()` to clear stale cached positions so modules are placed at their saved coordinates instead of reusing old ones
- **Fix `needs_reposition` leak across project loads** — `PatchEditor::clear()` now also clears the `needs_reposition` set, preventing stale module IDs from triggering unwanted repositioning after loading a new project

## [0.231.0] - 2026-03-12
### MCP client identity, module header gradient, octave label fix
- **MCP session registry with client identity** — replaced atomic session counter with `McpSessionRegistry` that tracks connected clients by name, version, and MCP protocol version; tooltip now lists each connected client (e.g. "claude-code v1.2.3 (MCP 2025-06-18)")
- **Module header gradient tint** — subtle diagonal gradient from top-left (module category color at 12% opacity) to transparent in the module header area, giving each module type a distinct visual identity
- **Octave label font size** — changed "Octave: +0" label above piano keyboard to use small font, matching surrounding UI elements

## [0.230.0] - 2026-03-11
### AWE top bar menu, English presets, extracted preset helper
- **AWE status icon in top bar** — new surround sound icon between OSC and project name, showing enabled/disabled state with preset name on hover
- **AWE preset dropdown menu** — click the AWE icon to select presets directly from the top bar (like MIDI port selector), with "Off" option at top, standard presets, and "Extreme" section
- **All AWE presets renamed to English** — 36 presets translated from Swedish to English (names and descriptions), e.g. "Katedral" → "Cathedral", "Glaslabyrint" → "Glass Labyrinth"
- **Extracted `apply_awe_preset()` helper** — shared function in `awe_view.rs` used by both the AWE view and the top bar menu, replacing ~80 lines of duplicated preset-apply code
- **Removed "← Rack" button from AWE toolbar** — navigation button removed along with unused `active_view` parameter from `draw_awe_view()`

## [0.229.0] - 2026-03-11
### Rack UI overhaul: patch metadata, effect zones, port shapes, and UX fixes
- **Auto Layout manual only** — removed automatic layout on startup and MCP reconciliation; Auto Layout now only runs when explicitly chosen from the right-click context menu
- **Patch saves author info** — patches now embed full author metadata (name, email, website, license) from Settings as a JSON object, with backward-compatible deserialization from legacy string format
- **Patch saves instrument name** — patch name is set from the active instrument name instead of being empty
- **Patch saves canvas size** — `canvas_size` field in `PatchSettings` preserves scroll area dimensions; restored on load for correct layout, cleared on Auto Layout and module removal so canvas can shrink
- **Position as object** — module and group positions serialized as `{"x": ..., "y": ...}` instead of arrays; backward-compatible deserialization from `[x, y]`
- **Grid size reduced** — rack grid changed from 50px to 32px for more compact module layouts; `auto_layout.rs` now references `patch_editor::GRID_SIZE` instead of duplicating the constant
- **Fix quit dialog loop** — "Don't Save" in the unsaved-changes dialog now clears the dirty flag before sending close, preventing infinite dialog reopening
- **Effect zone background** — effect and visualizer modules get a tinted background zone with border and "Effect Chain" label in the rack view
- **Distinct port shapes** — Audio=circle, Control/CV=diamond, Gate=square, MIDI=hexagon for instant visual port type identification
- **Port hover labels** — hovering a port shows its name and type (e.g. "cutoff_cv (cv)") on a tooltip layer above all modules
- **Port hover glow** — brighter glow ring on port hover for better feedback
- **`PatchAuthor` struct** — new serializable author type with `From<&str>` for built-in patches
- **`Position` struct** — new serializable 2D position type replacing `(f32, f32)` tuples

## [0.228.0] - 2026-03-09
### Fix silent project loading and parameter persistence architecture
- **Fix StereoOutput module_type** — return `ModuleType::StereoOutput` instead of `Mixer`, allowing graph evaluation to find the output node
- **Fix StereoOutput parameter names** — rename "Master Level" → "Master" and "Limiter" → "Limit" to match existing JSON project files
- **Stable parameter IDs (type_id)** — add `type_id` field to `ParameterDescriptor`, separating the persistent JSON key from the display name; display names can now be renamed freely without breaking saved projects
- **Snake_case parameter keys** — all parameter `type_id` values use snake_case (e.g. `"velocity_sens"`, `"attack_curve"`); deserialization falls back to display name matching for backward compatibility with older project files
- **Remove debug prints** — remove `[INST]` and `[SEQ]` eprintln from audio engine
- **Fix visualizer build** — remove unused `version_warned` field from `SynthTelemetry`

## [0.227.0] - 2026-03-09
### Audio engine performance and visualizer upgrade
- **Audio engine: connection lookup cache** — build `incoming_map` on topology change instead of O(M×C) scan per module per frame, eliminating ~100K comparisons/frame with complex patches
- **Audio engine: cached output module** — resolve output module ID once on graph change, not every `process()` call
- **Audio engine: eliminate per-frame allocations** — move mod source gathering into `ModuleGraph::gather_mod_source_values()` (avoids `collect::<Vec<_>>()`), pre-allocate mod matrix slots buffer on Voice
- **Visualizer: bounded OSC channel** — background `osc-reader` thread with `crossbeam_channel::bounded(100)`, drops packets when queue is full to prevent memory buildup during frame drops
- **Visualizer: TonyMcMapface tonemapping** — adds `Tonemapping::TonyMcMapface` for filmic color rendering
- **Visualizer: asymmetric easing** — FFT bars, waveform ring, and RMS light now use instant attack / smooth exponential decay for snappy transients without flicker
- **Visualizer: time-based centroid smoothing** — fixed frame-rate-dependent centroid smoothing in centroid_nebula to use proper `exp(-rate * dt)` interpolation
- **Visualizer: terrain mesh smoothing** — pulse_terrain tiles now lerp toward target height instead of jumping directly, eliminating per-frame vibration
- **Visualizer: per-theme floor PBR** — separate `floor_metallic`/`floor_roughness` per theme (Glass: mirror, Metal: brushed, Synthwave: reflective neon, Void: matte black)

## [0.226.0] - 2026-03-06
### Generative geometry effects and camera enhancements
- **Voronoi Shatter** (terrain) — ground grid of cells that split and tumble outward on spectral flux spikes, reassembling during calm passages
- **FFT Terrain** (terrain) — 16×8 grid of vertical pillars whose height is driven by FFT bins, with frequency→hue coloring (bass=red, highs=violet)
- **Reaction Diffusion** (ambient) — 12×12 sphere grid simulating Gray-Scott reaction-diffusion; spectral centroid controls feed rate, RMS controls kill rate, creating organic Turing patterns
- **Note Tree** (hero) — L-system inspired branching cylinders that grow on note events and wither during silence; voice count controls branch density, RMS drives sway
- **Dolly-zoom** on bass drops — FOV widens while camera pushes closer on low-FFT spikes
- **OSC `/viz/camera/mode`** — remote camera mode switching via OSC string argument
- 3 new preset scenes: Earthquake, Spectrum City, Living Forest

## [0.225.0] - 2026-03-06
### Screenshot, fullscreen, zoom, and keyboard shortcuts in debug HUD
- **Screenshot** (P key) — saves PNG to working directory via Bevy's `Screenshot` + `save_to_disk`
- **Fullscreen toggle** (F key) — switches between windowed and borderless fullscreen
- **Camera zoom** (Up/Down arrows) — smooth zoom in/out (5–60 units), camera height scales with distance
- **Shortcuts help in debug HUD** — F12 overlay now shows all keyboard shortcuts (Left/Right, Up/Down, R, T/Shift+T, F, P, F12)

## [0.224.0] - 2026-03-06
### Per-instrument colors and frequency-aware FFT hue mapping
- **Per-instrument color layers** — `category_hue_offset()` helper maps `InstrumentCategory` to distinct hue offsets (Drums→red, Bass→blue, Lead→gold, etc.); applied to particles, velocity_meteors, and harmonic_ribbons so different instruments produce visually distinct colors
- **Nonlinear frequency→hue mapping** — `band_frequency_hue()` uses sqrt curve giving bass more hue space (bass→red, mids→green, highs→violet); applied to fft_bars, spectral_waterfall, spectral_cathedral
- **`HueMaterialConfig.frequency_mapped`** flag — enables nonlinear band coloring in the shared material system; fft_bars opts in, note-based effects remain linear
- 3 new unit tests for category hue offsets and band frequency mapping

## [0.223.0] - 2026-03-06
### Telemetry-driven effects: velocity, voice count, peak, CC/pitch bend, transport
- **Velocity → brightness/size** — rms_light flashes on high-velocity notes, spectral_cathedral breathing boosted by velocity, pulse_terrain bass response amplified by recent velocity
- **Voice count → visual density** — centroid_nebula energy scales with voice count, particles burst size increases with more voices, fractal_pulse pulse amplitude boosted by voice count
- **Peak levels → transient flashes** — rms_light and beat_pulse react to peak threshold crossings with brightness spikes
- **CC/Pitch bend modulation** — pitch bend widens harmonic_ribbons wave amplitude, adds fold angle to spectral_origami; aftertouch widens neon_calligraphy strokes; pitch bend stretches calligraphy strokes
- **Transport state → animation speed** — pulse_terrain, fractal_pulse, spectral_origami slow to 15% speed when transport is stopped

## [0.222.0] - 2026-03-06
### Telemetry-driven effects: centroid hue, flux spikes, beat phase
- **Theme telemetry-reactive parameters** — ThemeConfig/ThemeMaterialPolicy extended with 7 new fields: centroid_hue_low/high, flux_burst_hue, flux_intensity_scale, beat_pulse_strength, peak_flash_hue, rms_emissive_scale — all lerped during theme transitions via ThemePolicySnapshot
- **CC/Pitch bend/Aftertouch storage** — SynthTelemetry now stores MIDI CC events, pitch bend (-1..1), and aftertouch (0..1) from `/synth/event/cc` (previously discarded)
- **telemetry_color helpers** — new `visuals/telemetry_color.rs` with `centroid_to_hue()`, `flux_emissive_boost()`, `beat_pulse_factor()`, `rms_to_emissive()`, `peak_exceeds_threshold()` + 10 unit tests
- **Centroid → hue shift** in 16 effects — spectral centroid drives palette hue via theme-defined ranges (e.g., Ember 0°–60°, Arctic 180°–240°, Void monochrome)
- **Flux → emissive spikes** in 12 material effects + rms_light + beat_pulse — spectral flux momentarily boosts emissive intensity, scaled by theme's flux_intensity_scale
- **Beat phase → pulsing** in fft_bars (bar height), fractal_pulse (ring scale), spectral_cathedral (arch breathing), pulse_terrain (ripple amplitude) — all scaled by theme's beat_pulse_strength
- **HueMaterialTracker** — replaces 3 separate Local params (last_fade, last_hue_offset, last_policy_version) with a single tracker struct including emissive_boost change detection

## [0.221.0] - 2026-03-06
### Theme material polish — metallic/roughness and ThemeRuntime
- **ThemeRuntime resource** — cached per-frame ambient brightness and key light intensity, replacing per-frame theme registry lookups in `beat_pulse` and `rms_light`
- **Metallic/roughness in ThemeMaterialPolicy** — themes now drive metallic and roughness on all hue-bucketed materials, floor, particles, and instrument cubes
- **Version-tracked policy updates** — `ThemeMaterialPolicy.version` counter bumped only on meaningful changes; all 15 effect material update systems skip work when policy is unchanged
- **`update_hue_materials_for_fade`** — accepts `last_policy_version` to trigger material refresh on theme transitions (not just fade changes)
- **Floor material** — metallic and roughness now lerped during theme transitions alongside color

## [0.220.0] - 2026-03-06
### Visualizer theme system
- **Theme system** -- 8 visual themes (Neon, Metal, Glass, Space, Synthwave, Ember, Arctic, Void) that swap lighting, bloom, material properties, and floor color
- **ThemeId enum** -- with `ALL` array, `next()`/`prev()` cycling, `name()` method
- **ThemeConfig** -- per-theme configuration for ambient/key/rim light, bloom, emissive multiplier, saturation/lightness offsets, metallic/roughness, floor color
- **ThemeRegistry resource** -- stores all theme configs; `ThemeState` resource tracks active theme and transition progress
- **ThemeMaterialPolicy resource** -- drives hue-bucketed material creation with theme-aware saturation, lightness, and emissive offsets
- **Smooth transitions** -- smoothstep-interpolated crossfade at 3 units/sec for all visual properties (lighting, bloom, materials, floor color)
- **Keyboard controls** -- `T` for next theme, `Shift+T` for previous theme
- **RmsLight theme integration** -- key light intensity multiplier from active theme instead of hardcoded 200,000
- **BeatPulse theme integration** -- base ambient brightness from active theme instead of hardcoded constant
- **FloorEntity / RimLight markers** -- component markers for theme system to update floor material and rim light
- **`create_hue_materials` / `update_hue_materials_for_fade`** -- now accept `ThemeMaterialPolicy` for theme-aware material offsets

## [0.219.0] - 2026-03-06
### Recent projects and dirty state tracking
- **Recent projects** -- `AppSettings` stores last 10 opened project paths; File > Recent Projects submenu with click-to-load and Clear Recent
- **Dirty state tracking** -- `dirty` flag set on module/connection/parameter/instrument/sequencer changes; cleared on save/load/new project
- **Unsaved changes dialog** -- confirmation dialog (Save / Don't Save / Cancel) shown before New Project, Open Project, loading recent projects, and Quit when dirty
- **Window title** -- shows `Pertylizer - <project> *` when unsaved changes exist
- **Close intercept** -- window close event intercepted when dirty, showing confirmation dialog instead
- **`PatchEditorResult::has_mutations()`** -- helper method for detecting any change in the patch editor result
- **`InstrumentRackResult::mutated`** -- tracks instrument add/remove/volume/pan changes
- **Settings backward compat** -- `recent_projects` field uses `#[serde(default)]` for seamless settings file upgrades

## [0.218.0] - 2026-03-06
### Undo/redo system
- **UndoManager** — stack-based undo/redo system in new `undo.rs` module with 100-action history limit
- **Sequencer note undo** — add, delete, move, transpose, resize, and paste operations are all undoable
- **Connection undo** — add/remove connections in the patch editor are undoable
- **Composite actions** — multi-note operations (transpose all selected, delete all selected, paste) grouped as single undo steps
- **Keyboard shortcuts** — Ctrl+Z for undo, Ctrl+Shift+Z for redo (global, works in all views)
- **Edit menu** — Undo/Redo menu items with greyed-out state when stacks are empty
- **Module position undo** — MoveModule action variant with `set_module_position()` API on PatchEditor
- **Inverse computation** — automatic inverse generation for all action types (add↔remove, old↔new values)

## [0.217.0] - 2026-03-06
### Offline WAV export
- **Export WAV** — render entire project to WAV file, faster than realtime
- **Export dialog** — File menu > Export WAV... with sample rate (44.1/48/96 kHz), bit depth (16/24/32-float), duration, and tail time settings
- **Progress bar** — real-time progress display with cancel button during rendering
- **Background rendering** — export runs in a separate thread, does not block the GUI or audio
- **Standalone engine** — creates a fresh SynthEngine from the project snapshot, loads all instruments/modules/connections/AWE state
- **`audio/export.rs`** — `ExportConfig`, `ExportProgress`, `start_export()`, `render_to_wav()` with hound WAV writer
- **`gui/export_dialog.rs`** — egui dialog with settings grid, estimated file size, and progress UI

## [0.216.0] - 2026-03-06
### Module copy/paste
- **Clipboard system** — new `gui/clipboard.rs` for in-memory module copy/paste
- **Copy selected modules** (Ctrl+C) — captures module states with parameters and positions
- **Paste modules** (Ctrl+V) — creates new instances with fresh IDs, applies stored parameters, remaps internal connections
- **Duplicate modules** (Ctrl+D) — copy + paste at 50px offset in one step
- **Multi-module support** — copies selection of modules and their internal connections (connections between selected modules are preserved)
- **Edit menu** — Copy, Paste, Duplicate items with keyboard shortcut hints
- **PatchEditor API** — `effective_selection()`, `extract_module_states()`, `internal_connections()`, `select_modules()`
- **`paste_clipboard_modules()`** in `patch_bridge.rs` — handles all module types including visualizers (SignalMonitor, Oscilloscope, LevelMeter, SpectrumAnalyzer)

## [0.215.0] - 2026-03-06
### Shared OSC protocol constants
- **New crate `synth_osc_protocol`** — minimal zero-dependency crate containing all OSC address constants and `PROTOCOL_VERSION`
- **`synth_osc` re-exports** — `addresses.rs` now re-exports from `synth_osc_protocol`, eliminating duplication
- **Visualizer uses shared constants** — `osc_receiver.rs` replaces 17 hardcoded address strings with `synth_osc_protocol::addresses::*`
- **Shared protocol version** — both sender (`synth_osc`) and receiver (visualizer) reference `synth_osc_protocol::PROTOCOL_VERSION`

## [0.214.0] - 2026-03-06
### Visualizer performance optimization
- **Disabled shadow maps** — removed point light shadow rendering (6 extra cube-face passes per frame)
- **Shared hue-bucketed materials** — all effects now use 8–16 shared material buckets instead of per-entity unique materials, enabling Bevy draw-call batching
- **Extracted material helpers** — `HueMaterialConfig`, `create_hue_materials()`, `update_hue_materials_for_fade()` in `effects.rs` eliminate ~150 lines of duplicated material setup
- **Scale-based fade** — per-entity visibility controlled via transform scale instead of emissive material mutation (avoids GPU re-uploads)
- **Removed `AlphaMode::Blend`** — phase_rings no longer disables instancing/batching
- **Reduced centroid_nebula particles** — 2000 → 500 particles
- **Material update gating** — materials only re-uploaded when fade changes by more than `FADE_EPSILON`
- **Fixed velocity_meteors bug** — exponential shrink from `transform.scale *= scale` → `transform.scale = Vec3::splat(life_pct)`
- **Pre-allocated mesh resources** — chord_bloom and harmonic_ribbons avoid per-spawn mesh creation
- **Updated docs** — TODO.md and osc-telemetry-plan.md reflect completed Phase 3 and all 13 creative effects

## [0.213.0] - 2026-03-05
### OSC/MCP defaults, Bevy 3D visualizer with bloom and beat-sync
- **OSC telemetry enabled by default** — no longer requires `--osc` flag; use `--no-osc` to disable
- **MCP server enabled by default** — `--features mcp` no longer needed (was already default feature)
- **Bevy 3D visualizer** (`visualizer/`) — separate Bevy 0.18 project receiving OSC telemetry over UDP:
  - FFT bar visualization (128 cubes with hue gradient, smooth lerp attack/decay)
  - RMS-driven point light (intensity tracks audio level)
  - Note-flash emissive sphere (hue from MIDI note, brightness from velocity)
  - Orbital camera (slow rotation around scene)
  - Bloom post-processing (HDR glow on emissive materials)
  - Beat-synced pulse (ground plane glow + ambient light boost on beat crossings, stronger on downbeats, frame-rate-independent decay)
- **Workspace cleanup** — `visualizer/` excluded from main workspace (separate Bevy dependency graph)
- **Updated README** — added OSC, visualizer, and `synth_osc` crate to features, tech stack, and crate table
- **rosc bumped to 0.11.4**

## [0.212.0] - 2026-03-05
### DSP abstractions and effects deduplication
- **`InputReader` abstraction** — zero-cost port reader (`inputs.reader(PortName, default)`) that eliminates `.map(|b| b[i]).unwrap_or(default)` boilerplate across all modules
- **`StereoSample` helpers** — `read_frame()`, `write_frame()` for interleaved buffer I/O, `blend()` for dry/wet mixing; replaces duplicated frame-reading and mix code in every stereo effect
- **`BufferIndex::read_interpolated()`** — shared circular buffer interpolation, replacing per-module copies in BBD delay, chorus, flanger, ensemble chorus, etc.
- **`Hertz::to_exp_coeff()`** — one-pole filter coefficient helper, deduplicates `(-TAU * freq / sr).exp()` pattern
- **`BiquadCoeffs::biquad_precompute()`** — shared omega/alpha computation for all biquad filter types
- **Removed `flush_denormals()` methods** — `FluidFilter`, `ScreamerFilter`, and other filters no longer need manual denormal flushing (handled by `FilterState`)
- **Effects cleanup** — all 20+ effects refactored to use new shared abstractions, removing ~600 lines of duplicated code
- **TODO updates** — added newtype arithmetic refinements backlog, linked AWE improvement findings

## [0.211.0] - 2026-03-05
### Deep newtype enforcement pass 2
- **New `StepCount(u8)` type** — for step-based pattern counts in generative modules (Euclidean, Turing Machine)
- **Param enum newtype upgrades** — `EuclideanParam::Steps/Pulses/Rotation` → `StepCount`, `TuringMachineParam::Length` → `StepCount`, `ModalResonatorParam::Modes` → `VoiceCount`, `WavetableParam::Octave` → `Octaves`
- **synth_engine function signatures** — `click_generator`, `cpu_tracker`, `metering` params changed from raw `f32`/`usize` to `SampleRate`/`Gain`/`BlockSize`
- **synth_sequencer input types** — `base_octave` → `Octaves`, `semitones` → `Semitones`, `strength` → `NormalizedValue`, `SetTempo` → `Bpm`
- **Piano roll GUI types** — `DragState`, `ClipboardNote`, `PianoRollNote`, `SequencerViewState`, `AutomationPointSnapshot`, `PianoRollData` fields replaced with `PatternTick`/`Pitch`/`Duration`/`Tick`/`Velocity`/`NormalizedValue`
- **Code quality cleanup** — removed redundant clamping, fixed semantic type mismatches (`ClipboardNote.tick_offset` → `SeqDuration`), `SetMetronomeVolume` → `Gain`

## [0.210.0] - 2026-03-04
### Newtype enforcement, build optimization, project docs
- **Newtype pattern enforcement** — replaced raw primitives with domain types across all crates (47 files, ~140 violations fixed): `Gain`, `BipolarValue`, `NormalizedValue`, `Phase`, `Hertz`, `Seconds`, `FilterState`, `SampleCount`, `Amplitude`, `CpuUsage`, `Velocity`, `Bpm`, `Pitch`, `RowCount`, `TicksPerRow`, `RowIndex`, `Duration`, `BlockSize`, `SampleRate`, `SamplePosition`, `Semitones`, `VoiceCount`
- **Release profile optimization** — added `opt-level = 3` and `strip = true` for smaller, faster release binaries
- **CLAUDE.md improvements** — clarified newtype rules (domain type list is examples, not exhaustive), added testing, error handling, code organization, and GUI sections

## [0.209.0] - 2026-03-04
### Piano roll: swing, scale velocities, step entry
- **Swing/shuffle control** — adjustable swing amount (0-100%) with Apply button; offsets even subdivisions relative to the selected grid resolution
- **Scale velocities** — toolbar control with percentage factor (1-200%) and Scale button to multiply velocities of selected notes
- **Step entry mode** — keyboard piano keys (Z-M, Q-U) insert notes at cursor position and advance by grid step; note preview on insert; wraps at pattern end

## [0.208.0] - 2026-03-04
### Piano roll: copy/paste, shortcuts, quantize, velocity editing, improvements
- **Copy/paste/duplicate** — Ctrl+C/X/V/D for note clipboard operations; clipboard stores relative offsets for flexible pasting; Ctrl+D duplicates selection to the right
- **Duplicate pattern** — right-click a pattern placement in arrangement view to duplicate it with a copy placed immediately after
- **Keyboard shortcuts** — Ctrl+A select all, Arrow Up/Down transpose by semitone, Shift+Arrow transpose by octave, Space toggle play/pause
- **Quantization UI** — grid resolution selector (Auto/1/4/1/8/1/16/1/32) in toolbar, quantize button with adjustable strength, applies to selected notes
- **Velocity editing** — drag velocity bars in the velocity lane to edit note velocities in real-time; configurable default velocity for new notes via toolbar slider
- **Note length presets** — toolbar selector for draw-mode note length (Drag/1/4/1/8/1/16)
- **Note preview on click** — plays a short preview of the note sound when clicking or drawing notes in the piano roll
- **Humanize** — toolbar button applies random timing (±15 ticks) and velocity (±5%) offsets to selected notes
- **Step entry mode** — toggle in toolbar shows a magenta cursor line
- **Swing control** — state field added (UI placeholder)

## [0.207.0] - 2026-03-04
### Live note preview and loop during recording
- **Live note preview** — notes appear as orange rectangles in the piano roll immediately during recording, before being committed; held notes extend in real-time from start tick to playhead
- **Loop during recording** — playback automatically loops around the target pattern region when recording starts, enabling multiple overdub passes without stopping; loop state is saved and restored when recording ends
- **Recording preview events** — new `RecordingPreview` engine event sends completed and held notes to the GUI at buffer rate (~86Hz) via the existing lock-free ringbuffer
- **Mid-recording flush** — recorded notes are written to the pattern at each loop boundary so they play back on subsequent passes; first flush respects the user's overdub/replace setting, subsequent flushes always overdub
- **Piano roll 50/50 split** — piano roll now defaults to ~50% of available height instead of fixed 300px; the existing resizable divider allows dragging to any proportion

## [0.206.0] - 2026-03-04
### Recording quantization and overdub mode
- **Recording quantization** — "Q" button in transport bar cycles through Off → 1/4 → 1/8 → 1/16 → 1/32 note grid; recorded notes are snapped to the selected grid on input
- **Overdub mode** — "OVR" toggle in transport bar; when on (default), new recordings layer on top of existing pattern notes; when off, existing notes are cleared before writing
- Both settings are sent with the `ArmRecord` command and applied during capture/flush

## [0.205.0] - 2026-03-04
### Recording (MIDI & keyboard), metronome, count-in
- **Real-time recording** — arm recording for the opened piano roll pattern, play to start capturing keyboard/MIDI notes into the pattern with correct timing
- **Metronome click track** — audible sine wave clicks on beat boundaries (accented on beat 1), toggle with "M" button in transport bar
- **Count-in (pre-roll)** — 1 bar metronome count-in before recording begins, allowing the player to hear the tempo before notes are captured
- **Recording buffer** (`recording.rs`) — real-time safe state machine (Idle → Armed → CountIn → Capturing) with pre-allocated note storage, song-to-pattern tick conversion, and held note tracking
- **Click generator** (`click_generator.rs`) — generates short sine wave bursts (1200 Hz accented, 800 Hz normal) with exponential decay envelope, fully real-time safe
- **Transport bar recording controls** — record button (disabled when no pattern open, blinking when armed/count-in, solid red when capturing), metronome toggle, status indicator (ARM/COUNT-IN/REC)
- **Engine commands** — `ArmRecord`, `DisarmRecord`, `SetMetronome`, `SetMetronomeVolume` with transport state atomics for GUI access
- **Flush on stop** — recorded notes are written to the pattern with correct durations when playback stops; held notes get a 16th-note fallback duration

## [0.204.0] - 2026-03-02
### Voice leak fix, arrangement view improvements, example project
- **Fixed voice leak bug** — voices in `Releasing` state never transitioned back to `Idle`, causing CPU to spike to 100% as voices accumulated indefinitely. Added `is_release_done()` trait method on `PolyModule`, implemented in `Envelope` and `Mseg`, with `all_releases_done()` check on `ModuleGraph` to reclaim finished voices
- **Arrangement view fixes** — fixed right-click menu position, track highlight on right-click, double-click to create pattern, Ctrl+scroll timeline zoom, pattern miniature note preview, hover tooltip, track header alignment, drag-to-move placements with beat grid snap
- **Added `move_placement()`** to Song for repositioning pattern placements
- **Example project** — "Oxygene Dreams" JMJ-inspired 2.5-minute composition with 9 instruments, drums, bells, and vocal pad

## [0.203.0] - 2026-03-02
### CI improvements, console removal, public release prep
- **Removed console GUI backend** — simplified CLI to just `--headless` and `--help`
- **Renamed `--mcp` flag to `--headless`** — clearer meaning for running without GUI
- **CI enhancements** — cargo cache, security audit (`cargo audit`), MSRV check (Rust 1.93), binary size logging
- **Dependabot** — automatic weekly dependency update PRs for Cargo and GitHub Actions
- **Build status badge** in README
- **Auto-release** — CI creates GitHub Release with binaries and changelog when version is bumped
- **MIT LICENSE file** added
- **Repository metadata** — `repository` URL and `description` added to workspace Cargo.toml
- **Sensitive files removed from git** — `.claude/settings.local.json` and AI tool configs untracked
- **Git history cleaned** — author email replaced with GitHub no-reply address
- **Improved auto-layout** — deterministic cycle-breaking in topological sort
- **Removed empty benchmark placeholder**

## [0.202.0] - 2026-03-02
### GitHub Actions CI
- **CI workflow** — GitHub Actions builds and tests on Ubuntu, Fedora (container), macOS, and Windows
- **Release binaries uploaded** as downloadable artifacts for all four platforms
- **Cross-platform build.rs** — replaced Unix `date` shell command with pure Rust date calculation

## [0.201.0] - 2026-03-02
### Rename to Pertylizer
- **Project renamed** from `modular_synth` / `modular-synth` to `pertylizer` — crate, binary, data paths, MIDI client names, UI strings, and all documentation updated
- **Build date in startup banner** — `Pertylizer v0.201.0 (2026-03-02)` printed to stdout on launch
- **Swedish documentation removed** — ARCHITECTURE.md, MCP.md, plan docs, and all crate READMEs deleted (can be recreated in English)
- **history.md translated** to English
- **README.md rewritten** — accurate module/effect counts (35 voice modules, 21 effects, 60 patches, 79 MCP tools), highlights for AWE, Fractal Oscillator, granular synthesis, spectral processing, and MCP
- **AI disclaimer** added to README

## [0.200.0] - 2026-03-02
### Cable clipping, remixicon patch categories, built-in patch search
- **Cable clipping fix** — active/hovered cables now clip to scroll area (use `Painter::new()` with explicit clip rect instead of `layer_painter()` which defaulted to `Rect::EVERYTHING`)
- **Cables moved to `Order::Background`** — drag and hover cables use late-allocated Background layers so they render above modules but stay within the patch viewport
- **Remixicon icons for patch categories** — replaced Unicode emoji (🎹🎸🎵🌊🥁🎻🔬🌫️) with `egui_remixicon` icons that render reliably on all systems
- **Built-in patch browser with search** — "Load Built-in" dialog now shows patches grouped by category with a free-text search field filtering on name, description, and tags

## [0.199.0] - 2026-03-02
### Fractal Oscillator (Weierstrass function)
- New **FractalOsc** module — stereo additive oscillator based on the Weierstrass function
- 64 partials with iterative power computation (no `powf` in hot loop)
- Parameters: Roughness (a), Fractal Spacing (b), Dispersion, Stereo Spread, Level
- Anti-aliasing: partials above Nyquist automatically skipped
- Equal-power stereo panning (even partials left, odd partials right)
- Amplitude normalization for consistent output level
- 100% real-time safe: zero heap allocations in process loop
- Full integration: GUI palette, module factory, parameter system

## [0.198.0] - 2026-03-02
### MCP Resources: Module catalog and example patches
- Added MCP Resources capability alongside existing Tools
- **Module catalog** — each module type exposed as `synth://module-types/{type_key}` with ports and parameters
- **Example patches** — each example patch exposed as `synth://patches/{slug}` with full patch data (modules, connections, parameters)
- Resource templates for both URI patterns
- New `get_example_patch` bridge method for retrieving full patch structure data
- New types: `PatchResourceData`, `PatchModuleInfo`, `PatchParamInfo`, `PatchParamValue`
- Updated docs/MCP.md with Resources documentation

## [0.197.0] - 2026-03-01
### 10 new example patches covering 23 previously unused module types

**New patches:**
- **Ethereal Shimmer Pad** — AdditiveOsc + EnsembleChorus + ShimmerReverb + Mid/Side + EQ
- **Granular Cathedral** — GranularOsc + Convolver (Hall) + SpectralBlur + GranularFx + Limiter
- **Analog Dream Machine** — MSEG + Phaser + Flanger + BBD Delay + Compressor
- **Resonant Percussion** — MechanicalNoise + ModalResonator + ReverseGateReverb + EQ
- **Spectral Freeze Pad** — AdditiveOsc + PhaseVocoder + SpectralBlur + ShimmerReverb + Limiter
- **Granular Storm** — GranularOsc + GranularFx + Phaser + Mid/Side + Compressor
- **MSEG Crystal Lead** — MSEG + AdditiveOsc + Flanger + BBD Delay + Compressor
- **Euclidean Texture** — Euclidean + TuringMachine + RandomGates + PhaseVocoder + ReverseGateReverb
- **Pitch Following Drone** — PitchTracker + GranularOsc + ModalResonator + EnsembleChorus
- **Vintage Electric Piano** — MechanicalNoise + Convolver (Plate) + EQ + BBD Delay + EnsembleChorus + Compressor

All 23 non-visualizer unused modules now have at least one example patch.

## [0.196.0] - 2026-03-01
### New Project, MCP project tools, save/load bug fixes

**MCP project management:**
- **MCP `new_project` tool** — reset to empty project via MCP
- **MCP `save_project` tool** — save current project to a file path via MCP
- **MCP `load_project` tool** — load a project or patch file via MCP, replacing all state
- **`ProjectAction` enum** in `mcp_shared.rs` — New/Save/Load actions queued by MCP, executed by GUI
- **Condvar-based result signaling** — MCP bridge waits for GUI to process action with 5s timeout

**GUI:**
- **New Project** menu item in File menu — resets all instruments, song, and arrangement to empty state
- **`reset_to_new_project()`** — reuses `load_project_data()` with a default empty `ProjectFile`
- **`default_instrument_state()`** helper in `project.rs` — creates instrument 0 with empty patch

**Bug fixes:**
- **Fix parameters saved as defaults** — `create_patch_from_editor()` now reads actual engine parameter values from `SharedGraphState` instead of stale GUI-cached defaults, fixing projects that sounded wrong after save/load
- **Fix module ID collision across instruments** — `SharedGraphState.modules` HashMap key changed from `ModuleId` to `(InstrumentId, ModuleId)`, preventing instruments with overlapping module IDs (e.g. each having `osc-1`) from overwriting each other

## [0.195.0] - 2026-02-28
### Project save/load (full project persistence)

**Project file format:**
- **`ProjectFile`** — top-level JSON container with `file_type: "project"` discriminator
- **`InstrumentState`** — serializable instrument with patch, volume, pan, mute, solo, key range, transpose, oversampling
- **`GlobalProjectState`** — master volume, octave offset, glide time, AWE state
- **File type detection** — `detect_file_type()` peeks at JSON to auto-detect patch vs project files

**Save logic:**
- **`create_project_from_app()`** — builds `ProjectFile` from all instruments + song + global state
- **`create_patch_from_editor()`** — extracts patch from `PatchEditor` without global settings (per-instrument)

**Load logic:**
- **`load_project_data()`** — stops playback, removes all instruments, recreates from project file
- Preserves original instrument IDs so sequencer track references remain valid
- **`add_instrument_with_id()`** + **`reset_counters_for_instrument()`** on `SynthSession` for clean reload

**UI integration:**
- **File menu** — Open Project, Save Project, Save Project As (above existing patch items)
- **Smart open** — Open Project dialog auto-detects patch vs project files
- **Projects directory** in Settings dialog (alongside Patches dir)
- **`last_project_dir`** remembered in settings for file dialog convenience
- **`FileDialogMode::OpenProject`** / **`SaveProject`** variants

## [0.194.0] - 2026-02-28
### Built-in group templates, groups in example patches

**Built-in group templates:**
- **12 hardcoded group templates** organized by category (Voice, Effect, Utility, Tutorial)
- Voice: Basic Synth Voice, Dual Oscillator Voice, FM Pair
- Effect: Chorus + Reverb, Delay + Reverb, Distortion + EQ
- Utility: Filter Sweep, Vibrato LFO, Slow Modulation, Dual Envelope
- Tutorial: Subtractive 101, AM Synthesis
- **`GroupTemplate` builder methods** — `new()`, `add_module()`, `add_connection()`, `expose_input()`, `expose_output()`
- **`Patch::add_group()` helper** — add groups to patches with auto-positioned group box

**Template browser integration:**
- **`GroupTemplateSource` enum** — `BuiltIn(index)` / `File(path)` to distinguish built-in from user-saved templates
- **Template browser shows both** built-in and file-based templates, grouped by category
- **"Built-in" badge** on built-in templates in the browser
- **Category sections** in the browser (Voice, Effect, Utility, Tutorial, Other)

**Groups in example patches:**
- Added module groups to 5 patches: Deep Space Pad, String Ensemble, Acid Bass, Vintage Lead, FM Bell
- Groups showcase Voice/Modulation/FM Engine/Output groupings with colored borders

## [0.193.0] - 2026-02-28
### Group templates, MCP auto layout, settings expansion

**Group templates:**
- **Save Group as Template** — right-click a group header → "Save as Template" to export as JSON with metadata (name, description, category)
- **Group Template Browser** — insert saved templates via right-click on background → "Insert Template", with search filter and category display
- **Template instantiation** — full ID remapping so templates can be inserted multiple times without conflicts
- **`GroupTemplateManager`** — scans template directory, loads/saves group template JSON files
- **New types:** `GroupTemplate`, `GroupCategory` in patch.rs

**MCP auto layout:**
- **New `auto_layout` MCP tool** — triggers GUI auto-layout of modules via `pending_auto_layout` AtomicBool on `McpSharedState`
- GUI polls the flag each frame and applies layout with actual rendered sizes

**Settings expansion:**
- **`DirectorySettings`** in `AppSettings` — stores custom patches dir, last open/save dirs
- **File dialogs remember last directory** — Open/Save start in last-used directory
- **Directories section in Settings dialog** — shows patches dir (with Reset) and settings file path
- **Renamed "Close" to "Save & Close"** in Settings dialog

## [0.192.0] - 2026-02-28
### Settings expansion — directories & file dialog memory
- **Added `DirectorySettings`** to `AppSettings` — stores custom patches dir, last open dir, last save dir
- **File dialogs remember last directory** — Open and Save dialogs now start in the last-used directory
- **Directories section in Settings dialog** — shows patches directory (with Reset button) and settings file path
- **Renamed "Close" to "Save & Close"** in Settings dialog for clarity
- **Made `settings_path()` public** so the dialog can display it
- **Fallback chain** for initial directory: last used dir → custom patches dir → default patches dir

## [0.191.0] - 2026-02-28
### Group menu icon replaces right-click
- **Added ⋯ (more) icon** to both expanded and collapsed group headers for accessing rename, color, ungroup, and delete actions
- **Removed right-click context menu trigger** — menu is now accessible via the visible icon only
- **Removed expand/collapse from context menu** — dedicated +/− icons already handle this

## [0.190.0] - 2026-02-28
### Remove `PatchModuleType` — use `ModuleType` directly for patch serialization
- **Removed `PatchModuleType` enum** (~370 lines) — was a redundant mirror of `synth_core::ModuleType`
- **`ModuleState.module_type`** now uses `synth_core::ModuleType` directly with `#[serde(rename_all = "snake_case")]`
- **Eliminated per-type boilerplate**: `as_str()`, `prefix()`, `from_module_type()`, `to_module_type()`, `is_effect_chain_module()`, `is_visualizer()`, `hides_ports()` — all already existed on `ModuleType`
- **Updated all 49 example patches** to use `ModuleType` instead of `PatchModuleType`
- **Simplified `create_patch_from_rack`** — uses `module_id.module_type` directly (no conversion needed)
- **Removed dead `load_inline_signal_monitor`** function from patch_bridge
- Adding new module types now requires updating only `ModuleType` — no separate patch enum needed

## [0.189.0] - 2026-02-28
### 6 new audio effects
- **Ensemble Chorus** (`enc`) — Juno-style BBD chorus with 2-3 voices, inverted LFO phases for stereo spread, one-pole tone filter, BBD clock noise, mid/side width processing
- **Shimmer Reverb** (`shr`) — FDN reverb with pitch-shifted feedback via granular ring-buffer pitch shifter, configurable pre-delay and shimmer amount
- **Granular FX** (`gfx`) — Input-driven granular processor with stereo ring buffer, 64 pre-allocated Hann-windowed grains, position/pitch/pan spread, freeze mode
- **Spectral Blur** (`sbl`) — STFT-based spectral smearing with temporal IIR smoothing per bin, spectral FIR smoothing across bins, freeze mode
- **Modal Resonator** (`mdr`) — Bank of 16 biquad bandpass resonators at harmonic frequencies with inharmonicity spread, brightness control
- **Reverse/Gate Reverb** (`rgr`) — Buffer capture with reverse/gate/stutter playback modes, periodic or threshold triggering, envelope shaping
- All effects follow `AudioEffect` trait with type-safe parameters, pre-allocated buffers, RT-safe processing
- Full registration: parameter enums, `ModuleType`/`Param` wiring, `EffectType`, factory, patch serialization, GUI palette labels

## [0.188.0] - 2026-02-28
### TODO 2.4 — MCP connection status indicator
- **MCP status in top bar** — robot icon shows connection state next to MIDI selector
- **Three states**: green filled icon + session count when active, dim outline when listening (idle), red when not running
- **Hover tooltip** with detailed status text
- **Session tracking** via `Arc<AtomicUsize>` shared between `McpSharedState` and `SynthMcpServer` (inc on create, dec on drop)
- **Listening flag** via `AtomicBool` set when HTTP server starts

## [0.187.0] - 2026-02-27
### TODO 2.3 — Segmented control tab buttons for Rack/AWE/Seq
- **Segmented control** — view selector buttons replaced with custom-painted connected pill-shaped segments
- Active tab gets filled accent color background, inactive tabs show dim text
- Vertical dividers between inactive segments, pill rounding on outer edges
- Click interaction via `allocate_exact_size` + `Sense::click()` per segment

## [0.186.0] - 2026-02-27
### Complete TODO 1.5 — Sequencer track & pattern management from GUI
- **Track management** — Add/remove/rename tracks from GUI with "+" button and right-click context menu
- **Mute/Solo buttons** — per-track M/S toggle buttons in track headers
- **Pattern management** — right-click timeline to create new patterns, place existing patterns, remove placements, delete patterns
- **Pattern length editing** — "Set Length" submenu in placement context menu (1/2/4/8/16 bars)
- **Pattern rename** — double-click pattern name in piano roll toolbar, or right-click "Rename Pattern" on placement
- **Instrument assignment** — ComboBox dropdown in track header to assign instruments to tracks
- **Piano roll instrument display** — shows assigned instrument name `[InstrumentName]` in piano roll toolbar
- **Loop toggle** — loop button in transport bar using `SetLoop` engine command
- **`EngineCommand::SetLoop`** — new variant routed to `SequencerEngine::set_loop()`
- **Empty state improved** — "Empty song" view now shows "Add Track" button instead of "Use MCP..." message
- Arrangement view refactored: track headers use egui widgets (SidePanel) for interactivity, timeline remains painter-based

## [0.185.0] - 2026-02-27
### Switch to egui-remixicon for icons (TODO 3.2, phase 1)
- **`egui-remixicon` 0.33 added** — Remix Icon font registered as fallback in `setup_custom_fonts()`
- **All palette/context menu icons replaced** — 40+ emoji → Remix Icon via shared `palette_label()` function
- **Module header icons replaced** — source/sink indicators, connectivity status, power/bypass, close buttons
- **File menu icons replaced** — New Patch, Open Patch, Save, Load Built-in, Example Patches, Settings, Quit
- **MIDI dropdown + instrument rack** — piano icons, active/inactive radio buttons
- **Transport controls replaced** — Play, Pause, Stop, Skip Back in sequencer
- **Tab icons added** — Rack, AWE, Seq view buttons now have icons
- **Piano roll close button** — replaced "X" with Remix Icon
- All GUI emoji replaced — only `⚠` in console text output remains

## [0.184.0] - 2026-02-27
### Complete TODO 2.1 — Remove module toolbar, native context menus
- **Module toolbar removed** — `TopBottomPanel::top("toolbar")` row deleted, Glide slider + counts moved to menu bar
- **Native egui menu styling** — both background and port right-click menus now use `response.context_menu()` / `egui::Popup` with `PopupKind::Menu`, matching File/Help menu styling
- **Shared `palette_label()` function** — single source of truth for module icons, names, and colors across all context menus
- **Port context menu improved** — same icons, colors, and layout as the background "Add module" menu
- **Swedish strings translated** — "Ta bort sladd" → "Delete cable", "Stoppa in Signal Monitor" → "Insert Signal Monitor"
- **`ModulePalette` struct + 20+ dead `add_*_module` methods removed**

## [0.182.0] - 2026-02-27
### Cleanup: remove debug module, English CLAUDE.md & README
- **Debug module removed** (TODO 1.4) — `src/debug/` (GraphDebugger, VoiceDebugger, SequencerDebugger, SignalProbe, ~2100 lines), `debug-tools` feature, and example programs (`debug_graph.rs`, `midi_test.rs`) all removed. MCP tools provide the same inspection capabilities.
- **CLAUDE.md translated to English** — trimmed generic Rust advice, added English language policy
- **README.md rewritten** — added feature overview, tech stack, build instructions, workspace crate table

## [0.181.0] - 2026-02-27
### Keyboard area cleanup (TODO 2.2)
- **PANIC button moved to top menu bar** — right-aligned with red styling, always accessible
- **"KEYBOARD" label removed** — piano keys are self-explanatory
- **"Playing: Default" renamed to "Active: {name}"** — clearer instrument indication
- **Octave controls moved inline** — compact header row above keys, eliminates full-width control row
- **Gap between keys and visualizers fixed** — tighter layout

## [0.180.0] - 2026-02-27
### Persistent settings file
- **`AppSettings`** — new `io::settings` module with JSON config at `~/.local/share/modular-synth/settings.json`
- **Theme persistence** — selected theme restored on startup (no longer resets to Dark)
- **Window geometry** — size and position saved on exit, restored on next launch
- **Author info** — name, email, website, and license fields in Settings dialog
- Settings auto-saved on change and on exit
- `ThemePreset` now derives `Serialize`/`Deserialize`
- `SynthGuiConfig` carries `AppSettings` instead of hardcoded width/height

## [0.179.0] - 2026-02-27
### MCP default + English UI
- **MCP feature enabled by default** — `default = ["gui-egui", "mcp"]`, no longer requires `--features mcp`
- **All UI strings translated to English** — AWE view (materials, labels, tooltips, section headings), patch editor context menus
- **Comprehensive TODO list** added with prioritized roadmap (docs/TODO.md)

## [0.178.0] - 2026-02-26
### InstrumentMapping: Stable SeqInstrumentId ↔ InstrumentId
- **`InstrumentMapping`** — new struct in `synth_engine` that maps `SeqInstrumentId(u16)` ↔ `InstrumentId(u64)` stably
- **`route_sequencer_events`** — now uses mapping lookup instead of unstable vec index (`instrument.0 as usize`)
- **`collect_events_at_tick`** — track instrument now overrides note instrument during playback (track.instrument → note.instrument fallback)
- **`seq_instrument_id`** — new field in `InstrumentSnapshot` for GUI/MCP visibility
- **MCP `add_note`/`add_notes`** — optional `instrument_id` parameter (default 0, track instrument overrides during playback)
- Mapping is automatically updated on instrument creation/deletion
- Orphaned notes (deleted instrument) fall back to the first instrument

## [0.177.0] - 2026-02-26
### MCP: Complete API with 18 new tools
- **Automation CRUD** — `list_automation_lanes`, `get_automation_points`, `remove_automation_points`, `clear_automation_lane` + `curve` parameter (Linear/Step/Exponential/SCurve) on `add_automation_points`
- **Track control** — `set_track_volume`, `set_track_pan`, `set_track_mute`, `set_track_solo`, `rename_track`, `delete_track`
- **Pattern management** — `rename_pattern`, `set_pattern_length`, `duplicate_pattern`
- **Song metadata** — `set_song_author`, `set_song_time_signature`
- **Batch parameter** — `set_parameters` (set multiple module parameters in one call)
- **Automation in `set_song`/`create_patterns`** — patterns can now include automation inline
- Total 79 MCP tools (up from 61)

## [0.176.0] - 2026-02-26
### MCP: Streamable HTTP + Hybrid Resonator preset
- **MCP transport: TCP → Streamable HTTP** — Server now uses axum + rmcp Streamable HTTP on `http://127.0.0.1:9850/mcp`. Claude Code connects directly without bridge process.
- **Removed `synth-mcp-bridge`** — Stdio↔TCP proxy binary is no longer needed
- **Fix: MCP tools capability** — `ServerCapabilities::enable_tools()` is now advertised in initialize response, so Claude Code discovers the tools
- **New preset: Hybrid Resonator** (Experimental) — Layered hybrid voice with ring-modulated wavetable, bitwise math oscillator, body resonance and LFO/envelope modulation
- Updated `.mcp.json` to `"type": "http"` configuration
- Updated `mcp-call.py` helper to HTTP-based communication
- Updated documentation (ARCHITECTURE.md, MCP.md, SKILL.md)

## [0.175.0] - 2026-02-26
### Sequencer GUI — Automation in Piano Roll (Phase 5)
- **Automation zone** below velocity zone in piano roll (80px tall, toggled via dropdown)
- **Lane selector** (ComboBox) in toolbar: select `AutoInstrumentParam` for instrument 0 (Volume, Pan, Filter Cutoff, etc.)
- Active lanes (with points) marked with `*` in dropdown
- **Curve rendering** with pixel-by-pixel interpolation (Linear, Step, Exponential, SCurve)
- Flat extension before first / after last point
- Reference lines at 25%, 50%, 75%
- **Point interaction**: click → create point, right-click → remove point, drag → move point
- Ghost preview during drag with semi-transparent circle
- Orange circles with white border for automation points
- **Automation playback** in `SequencerEngine`: `collect_events_at_tick()` generates `SequencerEvent::Parameter` from automation lanes
- Deduplication via `last_automation_values` HashMap (emit only on change > 0.001)
- **Event routing** in `SynthEngine`: Volume → `Instrument::set_volume()`, Pan → `Instrument::set_pan()`
- `display_name()` on `AutoInstrumentParam` and `AutomationTarget` for GUI labels
- `AutoInstrumentParam::ALL` const array for GUI enumeration

## [0.174.0] - 2026-02-26
### Piano Roll Bug Fixes & Improvements
- **Fix: `move_note` reused NoteId** — `next_note_id` counter was incorrectly decremented, causing "copied" notes (synth_sequencer/pattern.rs)
- **Fix: Vertical drag now works** — `drag_started()` uses `press_origin()` instead of `interact_pointer_pos()` so hit-test occurs at click position, not after drag threshold
- **Fix: Relative grip when moving** — notes maintain position relative to cursor (grab offset), not snap to note start
- **Fix: Short-note resize zone proportional** — `min(RESIZE_GRAB_ZONE, note_width * 0.3)` so short notes can be moved
- **Fix: `quantize_tick` floor instead of nearest** — notes snap to grid line at/before click, not nearest
- **Fix: `y_to_pitch` clamp** — pitch clamped to visible range when dragging outside grid
- **Fix: Zero-duration guard** — `SeqDuration` always at least 1 tick
- Grid width now includes notes extending past pattern length
- Open-ended notes (`duration=None`) get explicit duration at drag start
- Expanded hit-rects for tiny notes (`< RESIZE_GRAB_ZONE`)

## [0.173.0] - 2026-02-26
### Sequencer GUI — Piano Roll Mouse Interaction (Phase 4)
- Click in grid (Draw tool) creates new notes with quantization to `RowResolution`
- Click on note (Select/Draw) selects the note, Shift+click toggles in selection
- Drag note body → move note (tick + pitch), visual ghost preview during drag
- Drag right edge → resize note (change duration)
- Selection rectangle (lasso) in Select tool: drag on empty area → select all notes in rectangle
- Delete/Backspace removes all selected notes, Escape clears selection
- Tool selector in toolbar: Select / Draw with active indicator
- Visual feedback: selected notes lighter blue + thicker border, hover highlight, ghost preview
- Cursor icons: Crosshair (Draw), PointingHand (hover note), ResizeEast (right edge)
- Edit Transactions: Song is mutated only on drag release (not during drag)
- `NoteId` included in `PianoRollNote` snapshot for hit-testing and commands
- `draw_piano_roll` now takes `song` and `view_state` for write lock and interaction state
- `allocate_rect` with `Sense::click_and_drag()` replaces `allocate_space`

## [0.172.0] - 2026-02-25
### Sequencer GUI — Piano Roll Read View (Phase 3)
- Piano roll opens on double-click on PatternPlacement in arrangement
- `SequencerViewState` with `opened_pattern` — persists between frames
- Double-click detection via `allocate_painter` with `Sense::click()` + rect hit-test
- Data collected in snapshot (`PianoRollData`) via short read-lock (RT-safe)
- Keyboard column (left): keys with C notes marked, black/white keys
- Note grid: horizontal pitch rows, vertical beat/sub-beat lines, black keys darker
- Notes as color-coded rectangles with alpha based on velocity
- Open-ended notes (duration=None) drawn to pattern end with fade indicator
- Velocity bars (bottom zone) with color gradient green→yellow→red
- Playhead marker (pattern-relative position)
- Resizable bottom panel (150-600px) with scroll in both directions
- Close button (X) in toolbar

## [0.171.0] - 2026-02-25
### Sequencer GUI — Arrangement View (Phase 2)
- Graphical arrangement timeline with scrollbar
- Track panel (left): names with color indicator, M/S flags
- Timeline ruler with bar numbers (1-based)
- Grid lines: strong at bar lines, weak at beats
- Pattern placements as color-coded rectangles (track color) with name and note count
- Playhead marker (vertical line + triangle in ruler) — moves in real-time during playback
- Culling: only visible placements are rendered
- Data collected in snapshot (short read-lock) before rendering (RT-safe)

## [0.170.0] - 2026-02-25
### Sequencer GUI — Transport & Read View (Phase 1)
- Transport bar with Play/Pause/Stop buttons, Go to Start
- Position display: Bar:Beat:Tick (1-based, monospace, updates in real-time)
- Tempo DragValue (20-300 BPM) — sends `EngineCommand::SetTempo`
- Time signature display from Song
- Status indicators: PLAYING (green) / PAUSED (yellow) / STOPPED (gray)
- Song info: name, tracks (with color/mute/solo), patterns (name, notes, length)
- `ctx.request_repaint()` during playback for smooth position updates
- Transport reads atomic values from `TransportState` (lock-free)

## [0.169.0] - 2026-02-25
### Sequencer GUI — Integration and RT Safety (Phase 0)
- Song always available in GUI (not just with MCP feature) — `SynthGuiConfig.song`
- Song created in `main.rs` and shared with engine, GUI and MCP via same `Arc<RwLock<Song>>`
- `McpSharedState::with_song()` receives external Song instead of creating its own
- `SequencerEngine`: `song.read()` → `song.try_read()` for RT safety (never blocks audio thread)
- New `AppView::Sequencer` variant in navigation state
- New `SequencerGuiInput` (implements `InputSource`) — GUI command queue for sequencer
- Stub sequencer view shows Song info (name, tempo, pattern/track count)
- 3-way view switcher in header: Rack / AWE / Seq

## [0.168.0] - 2026-02-25
### Unified right-click menu (cable-aware)
- Cable and background context menus merged into a single menu
- Right-click on cable shows: "Delete cable", "Insert Signal Monitor", separator, "Insert module..." (with full module catalog)
- Selected module is inserted inline on the cable (from→new module→to)
- Exactly one cable hovered at a time (nearest, within 20px) — no overlap
- Hamburger icon on hovered cable removed — cable is still highlighted
- Port right-click unaffected

## [0.167.0] - 2026-02-25
### Right-click menu for adding modules in the patch editor
- Right-click on empty area in patch editor opens a hierarchical context menu with all module categories
- Selected module is placed at click position (instead of auto-layout)
- Same menu structure as top bar palette: Oscillator, Filter, Envelope, LFO, VCA, Mixer, Effect, Visualizer, Modulation, Generative, Physical, Output, Mod Matrix
- Submenus for Oscillator, Effect, Visualizer, Modulation, Generative and Physical
- Existing right-click menus (cable, port) unaffected

## [0.166.0] - 2026-02-25
### Type safety: String → PortName throughout, GUI display of module/instrument ID

**Type safety (PortName refactor):**
- `PortId.port`: `String` → `PortName` (Copy, zero allocation). `PortId` now `Copy`.
- `ConnectionSnapshot` port fields: `String` → `PortName`. Now `Copy`.
- `PortDescriptor.name`: `String` → `PortName` — affects all ~29 modules.
- `GraphNode.outputs`: `HashMap<String, AudioBuffer>` → `HashMap<PortName, AudioBuffer>`.
- `PolyModule::process()` output parameter: `HashMap<String, ..>` → `HashMap<PortName, ..>`.
- `Session::connect/disconnect`: `String` → `impl Into<PortName>`.
- `ModuleGraph::get_module_output`: `&str` → `PortName`.
- GUI-internal types (`PendingConnection`, `QuickAddRequest`, `PortContextMenuState`, `port_positions`) → `PortName`.
- Debug types (`ConnectionInfo`, `ProbePoint`) → `PortName`. Now `Copy`.
- `ModuleStateSnapshot` levels/counters: `HashMap<String, ..>` → `HashMap<PortName, ..>`.
- `ModuleVisualState.port_states`: `HashMap<String, ..>` → `HashMap<PortName, ..>`.
- New PortName constants: `PITCH`, `PITCH_CV`, `ACCENT`.
- `Serialize`/`Deserialize` implemented for `PortName`.
- `PartialEq<&str>` implemented for `PortName`.

**GUI improvements:**
- Tooltip with ModuleId on hover over module title in patch editor.
- Tooltip with InstrumentId on hover over instrument name in instrument list.

## [0.165.0] - 2026-02-25
### 3 new example patches, PatchModuleType v0.162.0 support, parameter fixes in all 48 patches

**New example patches:**
- **LA Synth Pluck** (Experimental) — Roland D-50-style LA synthesis with percussive attack transient crossfading to filtered saw.
- **Vector Pad** (Pad) — 4 oscillators blended via VectorMixer with dual LFOs for constantly evolving timbre.
- **Spectral Drone** (Ambient & Texture) — Detuned sawtooth waves through FrequencyShifter for inharmonic metallic shimmer.

**New `PatchModuleType` variants:**
- `VectorMixer`, `LaSynth`, `PitchTracker`, `FrequencyShifter` — support in patch serialization, `to_module_type()`, `patch_bridge.rs` mapping.

**Bug fixes (parameter names in all 48 example patches):**
- `filter_mode()` saved `"filter_type"` → fixed to `"type"` (did not match the filter's actual parameter).
- `distortion_mode()` saved `"mode"` → fixed to `"type"` (Distortion params are called `Type`, not `Mode`).
- StereoOutput `param_f("master")` → `"master level"` in all patches (did not match `"Master Level"`).
- Mixer `param_f("level")` → `"master"` in 5 patches (Mixer has `Master`, not `Level`).
- Envelope: `attack_curve`→`atk_curve`, `decay_curve`→`dec_curve`, `release_curve`→`rel_curve`, `velocity_sensitivity`/`velocity_sens`→`vel_sens`.
- Filter: `key_tracking`→`key_track`, `cutoff_mod`/`env_amount`→`cv_amt`.

**Improvements:**
- **Fuzzy parameter matching** — `set_parameter()` now normalizes underscores to spaces during name matching (`input_1` matches `Input 1`).
- **Mixer descriptor** — Added `Input 1`–`Input 8` parameters that were missing in ModuleDescriptor. Fixes parameter setting via `apply_patch` and MCP.
- **48/48 example patches now load without warnings** via `apply_example_patch`.

## [0.164.0] - 2026-02-25
### MCP Batch patch building: build_instrument, build_instruments, apply_example_patch

**New MCP tools:**
- **`build_instrument`** — Build a complete instrument in ONE call: creates instrument, adds modules, sets parameters and wires cables. Modules referenced via 0-based array index in connections.
- **`build_instruments`** — Build MULTIPLE instruments in a single call (batch).
- **`apply_example_patch`** — Load a named example patch directly without GUI queue. Creates all modules, parameters and connections immediately. Creates new instrument if no instrument_id is provided.

**Bug fixes:**
- **Effect parameter routing** — MCP `set_parameter` now correctly sends `SetEffectParameter` for effect modules (Delay, Reverb, Chorus etc.) instead of always using `SetModuleParameter`.

**New architecture:**
- `SynthSession::set_parameter()` — GUI-independent parameter logic with correct effect/voice routing and `ParamValue::Choice` → f32 conversion.
- `SynthSession::apply_patch()` — GUI-independent patch loading that creates modules, applies parameters and connects. Skips visualizers (Oscilloscope, SignalMonitor etc.).
- `PatchModuleType::to_module_type()` — New conversion method from patch format to engine `ModuleType`.
- `ApplyPatchResult` — Result type with module_count, connection_count, module_ids and errors.

## [0.163.0] - 2026-02-25
### Per-instrument module IDs and GUI reconciliation for all instruments

**Bug fixes:**
- **Per-instrument module IDs** — Each instrument now has its own counters per module type. Instrument 0 can have `osc-1` and instrument 1 can also have `osc-1` (previously IDs were global: `osc-1`, `osc-2`, `osc-3` regardless of instrument).
- **GUI reconciliation for all instruments** — `reconcile_with_session()` now synchronizes modules and connections for ALL instruments, not just the active one. Modules and cables created via MCP on non-active instruments are visible immediately on instrument switch.

**Technical changes:**
- `SynthSession.counters` changed from `HashMap<ModuleType, u16>` to `HashMap<(InstrumentId, ModuleType), u16>`
- `SynthSession.registry` changed from `HashMap<ModuleId, RegistryEntry>` to `HashMap<(InstrumentId, ModuleId), ModuleDescriptor>`
- `InstrumentId::MASTER` sentinel constant for master bus effects
- `validate_port()` in MCP bridge now takes `instrument_id` for correct module lookup
- Clippy fixes: `collapsible_if`, `map_or` → `is_some_and`

## [0.162.0] - 2026-02-25
### 4 new modules: Frequency Shifter, Vector Mixer, LA Synth, Pitch Tracker

**New effect modules:**
- **Frequency Shifter** (`fsf`) — Bode frequency shifting with Hilbert transform pair (all-pass chains). Three modes: Up-shift, Down-shift, Stereo (up L / down R). Parameters: Shift (-1000 to +1000 Hz), Mix, Mode.

**New voice modules:**
- **Vector Mixer** (`vec`) — 4-corner XY vector mixing with equal-power bilinear interpolation. 4 audio inputs (A/B/C/D), X/Y CV modulation.
- **LA Synth** (`las`) — Linear Arithmetic synthesis (Roland-style). Generates attack transients (click, noise burst, pluck, hammer) that crossfade to sustain input. Parameters: Attack Type, Attack Time, Attack Level, X-Fade Time, Brightness.
- **Pitch Tracker** (`ptr`) — Autocorrelation-based pitch detection. Outputs: 1V/octave pitch CV and gate signal. Pre-allocated ring buffer (2048 samples), analysis every 512 samples.

## [0.160.0] - 2026-02-25
### GUI reconciliation for MCP instrument changes

- GUI now automatically updates when instruments are created, removed, renamed or changed via MCP
- New `reconcile_instruments()` in GUI — compares `instrument_snapshots` with GUI state every frame
- MCP-created instruments appear in instrument rack, MCP-deleted ones disappear
- Metadata (name, volume, pan, mute, solo) synced from engine to GUI
- `EngineCommand::RenameInstrument` — new engine command for renaming (replaces direct writing to shared state which was overwritten)
- `next_instrument_id` kept in sync between GUI and MCP

## [0.159.0] - 2026-02-25
### MCP: Multi-instrument support

**9 new MCP tools for instrument management (total 56):**

- `create_instrument` — Create new instrument with name
- `delete_instrument` — Delete instrument (not the default instrument ID 0)
- `rename_instrument` — Rename instrument
- `set_instrument_volume` — Set volume (0.0–2.0)
- `set_instrument_pan` — Set pan (-1.0–1.0)
- `set_instrument_mute` — Mute/unmute
- `set_instrument_solo` — Solo/unsolo
- `set_instrument_midi_channel` — MIDI channel (1–16)
- `set_instrument_enabled` — Enable/disable

**Architecture changes:**

- `SharedGraphState` tags snapshots with `instrument_id` — modules/connections per instrument
- `InstrumentSnapshot` in `EngineState` — name, channel, volume, pan, mute, solo
- `SynthSession` — instrument lifecycle (add/remove/rename/configure) + per-instrument module registry
- `SynthBridge` trait extended with 9 new instrument methods
- `AppSynthBridge` — all `instrument_id != 0` guards removed, validates via snapshots
- GUI reconciliation filters on active instrument
- `InstrumentInfo` extended with volume, pan, muted, solo fields
- Engine updates instrument snapshots on every instrument change

**Limitation removed:** "Only one instrument (ID 0) exposed" — multiple instruments now supported via MCP.

## [0.158.0] - 2026-02-25
### SynthSession — shared control layer for GUI and MCP

**New architecture:** Thread-safe `SynthSession` owns module lifecycle (create, remove, connect) and is used by both GUI and MCP.

**Changes:**
- `SynthSession` (`session.rs`) — new struct with `add_module()`, `remove_module()`, `connect()`, `disconnect()`, `clear_graph()`, queries
- MCP performs module operations directly via session (immediate feedback with ModuleId)
- MCP now works correctly in headless mode (`--mcp`) — previously completely broken
- GUI reconciles with session every frame — MCP-added modules appear automatically
- `module_factory.rs` moved from `gui/` to crate root (zero GUI dependencies)
- `patch_bridge.rs` reduced from ~900 lines of duplicated factory logic to ~80 lines via `session.add_module_with_id()`
- `PendingMcpOp` and `pending_ops` removed — replaced by direct session calls
- Eliminates code duplication between GUI, MCP and patch loading

## [0.157.0] - 2026-02-25
### MCP: Batch operations for the sequencer

**8 new batch MCP tools (total 47):**

*Batch notes:*
- `add_notes` — Add N notes to a pattern in one call
- `update_notes` — Update N notes in a pattern in one call
- `replace_notes` — Clear + add N notes (full overwrite)
- `clear_pattern` — Clear all notes from a pattern

*Batch creation:*
- `create_patterns` — Create N patterns with optional inline notes
- `create_tracks` — Create N tracks in one call

*Batch arrangement:*
- `place_patterns` — Place N patterns in the arrangement

*Full song:*
- `set_song` — Build an entire song in one call (patterns + tracks + notes + arrangement)

**Design decisions:**
- Partial success: each batch result reports per-item success/failure (no rollback)
- `set_song` uses array indices (0-based) for placements, not IDs — returns mapping index → ID
- Existing single-item tools retained

**New types:**
- `BatchItemResult`, `BatchResult`, `SetSongResult` — response types
- Bridge structs: `BridgeNoteData`, `BridgeNoteUpdate`, `BridgePatternData`, `BridgeTrackData`, `BridgePlacementData`, `BridgeSongPlacement`

## [0.156.0] - 2026-02-25
### MCP: Sequencer tools — Song, Pattern, Note, Track, Transport

**New shared Song via `McpSharedState`:**
- `Arc<RwLock<Song>>` created in `McpSharedState` and sent to engine via `SetSong` at startup
- MCP edits Song directly via RwLock, SequencerEngine reads it in the process loop

**17 new MCP tools (total 39):**

*Song:*
- `get_song_info` — Name, tempo, time signature, length, pattern/track count
- `set_song_tempo` — Change tempo (BPM)
- `set_song_name` — Change song name

*Patterns:*
- `list_patterns` — List all patterns with length and note count
- `create_pattern` — Create new pattern (name, length in beats)
- `delete_pattern` — Delete pattern (and all its placements)

*Notes:*
- `list_notes` — List all notes in a pattern
- `add_note` — Add note (pitch, start_beat, duration_beats, velocity)
- `remove_note` — Remove note
- `update_note` — Update note (optional fields: pitch, start, duration, velocity)

*Tracks:*
- `list_tracks` — List all tracks with instrument, volume, mute/solo
- `create_track` — Create new track (name, optional instrument)

*Arrangement:*
- `place_pattern` — Place pattern on track at beat position
- `remove_placement` — Remove placement
- `list_arrangement` — List all placements

*Transport:*
- `seq_play` — Start sequencer playback
- `seq_stop` — Stop sequencer
- `seq_seek` — Seek to beat position

**Time conversion:** MCP API uses beats (float) — internally: `beat × 960 = tick`.

**New error types:** `PatternNotFound`, `NoteNotFound`, `TrackNotFound`, `SongLockPoisoned`.

## [0.155.0] - 2026-02-25
### New example patch: Moog Resonant Sweep

**New patch:** "Moog Resonant Sweep" — fat Moog-inspired lead/bass with:
- Dual detuned sawtooth oscs + sub-osc one octave down
- Ladder filter with high resonance (0.55), 6000Hz envelope sweep, drive 1.8
- LFO wobble on filter (1.8Hz triangle)
- Chorus + reverb effects

Added to Lead category in patch browser.

## [0.154.0] - 2026-02-25
### MCP: Port validation in connect/disconnect

**Bug fix:**
- `connect`/`disconnect` now validates module IDs and port names against `ModuleDescriptor` before queueing the operation. Returns `PortNotFound` error with module and port name if the port does not exist (instead of silent "OK").

**New error variant:** `McpBridgeError::PortNotFound { module, port }` in `synth_mcp::error`.

## [0.153.0] - 2026-02-25
### MCP: Effect modules via add_module/remove_module

**Improvements:**
- `add_module` now supports effect modules (chr, rev, dly, dist, phs, fln, comp, eq, ws, bbd, ms, lim, conv, pvoc) — not just voice modules
- `remove_module` now sends the correct `EngineCommand` depending on module category (Effect → `RemoveEffect`, Visualizer → `RemoveVisualizer`, otherwise `RemoveModule`)

**New factory function:** `create_effect(ModuleType)` in `module_factory.rs` — creates effect instances (`Box<dyn AudioEffect>` + `ModuleDescriptor`), parallel to `create_voice_module`. `get_descriptor()` simplified to use it.

## [0.152.0] - 2026-02-24
### MCP: clear_graph + cleanup

**New MCP tool (total 21):**
- `clear_graph` — Clears the entire voice graph for an instrument (removes all modules, effects, visualizers and cables). Useful for starting from scratch.

**Removed:**
- `signal_level` field in `ConnectionSnapshot` and `ConnectionInfo` (GUI uses separate `CableVisualState` system, MCP field was redundant)

## [0.151.0] - 2026-02-24
### MCP: Module management + bug fixes

**5 new MCP tools (total 20):**
- `list_module_types` — Lists all available module types with ports and parameters
- `add_module` — Adds a module to the voice graph (visible in GUI next frame)
- `remove_module` — Removes a module and all its cables
- `connect` — Connects two module ports with a cable
- `disconnect` — Removes a cable between two module ports

**New module factory:** `module_factory.rs` centralizes creation of module instances from `ModuleType` (25 voice modules + 14 effects + 3 visualizers).

**New shared state:** `PendingMcpOp` — queue for MCP→GUI operations (AddModule, RemoveModule, Connect, Disconnect), polled every frame by GUI.

**Bug fixes:**
- `effect_count` in `InstrumentInfo` now reads from `EngineState` (was hardcoded to 0)

**New bridge types:** `ModuleTypeInfo`, `PendingMcpOp`, `InvalidModuleType` error variant

## [0.150.0] - 2026-02-24
### Fix bugs in example patches

**10 patches fixed** via MCP-driven testing (all 44 now pass without errors):

- **8 patches with redundant modulation:** Removed cables that duplicated ModMatrix routing (env/lfo → filter cutoff_cv). Affected: Fluid Keys, Acid Bass, Screamer Lead, Waveshaper Lead, Unison Supersaw, Glitch Pad, Fluid Pad, Unison Sync Lead
- **Unison Sync Lead:** Waveshaper is an effect chain module — cannot be connected inline in voice graph. Fixed: osc→flt directly, waveshaper applied automatically via effect chain
- **FM Bell:** Oscillator has no "cv" port. Fixed: added amp-2 to envelope the modulator signal (osc-2 → amp-1 → osc-1 fm), correct FM depth control with env-1
- **Warm Evolving:** lfo-2 was defined but never connected. Fixed: lfo-2 → flt-1 cutoff_cv (morph_cv does not exist as a filter port)

## [0.149.0] - 2026-02-24
### MCP: Example patches + UI snapshot

**3 new MCP tools (total 15):**
- `list_example_patches` — Lists all 45 example patches with category, description, tags, module/connection count
- `load_example_patch` — Loads an example patch by name (case-insensitive), GUI updates next frame
- `get_ui_snapshot` — Returns module positions, sizes, parameters, connections and overlap analysis

**New shared state: `McpSharedState`**
- `pending_patch: Mutex<Option<(Patch, String)>>` — MCP writes, GUI polls every frame
- `ui_layout: Mutex<UiLayoutData>` — GUI writes every frame, MCP reads on request
- Shared via `Arc` between `AppSynthBridge` and `SynthApp`

**New MCP types:** `ExamplePatchInfo`, `UiSnapshot`, `UiModuleInfo`, `UiConnectionInfo`, `UiOverlap`

## [0.148.0] - 2026-02-24
### MCP bug fixes and improved documentation

**5 bug fixes:**
- `list_instruments` and `get_engine_status` now work (Parameters<()> → NoParams struct)
- Module IDs use Display format (`osc-1`) instead of Debug (`ModuleId { module_type: Oscillator, instance: 1 }`)
- `format_param_display()` now matches case-insensitively — units display correctly (`"2.0 kHz"`, `"440.0 Hz"`)
- `get_graph_diagnostics` no longer reports StereoOutput as "signal dead-end"
- `module_type` in ModuleInfo uses `ModuleType::name()` instead of Debug format

**Documentation:** Updated `docs/MCP.md` with example session, known limitations, phase 2/3 planning, and creative use cases

## [0.147.0] - 2026-02-24
### MCP support (Model Context Protocol) for AI agent integration

**New crate: `synth_mcp`** — MCP server that lets AI agents inspect and control the synth:
- 11 tools: list_instruments, list_modules, get_module_info, get_connections, get_parameter, get_engine_status, get_graph_diagnostics, set_parameter, note_on, note_off
- `SynthBridge` trait for clean separation between MCP protocol and synth engine
- TCP server on port 9850 (GUI + MCP simultaneously) and stdio mode (`--mcp` headless)
- Stdio↔TCP bridge binary (`synth-mcp-bridge`) for Claude Code integration

**SharedGraphState connected to EngineState:**
- `EngineState.shared_graph` updated on topology changes and parameter changes
- Snapshot built from `Instrument.voice_graph()` on AddModule, RemoveModule, Connect, Disconnect, SetModuleParameter, SetVoiceParameter

**Feature flag:** `mcp` — behind feature flag, default builds unaffected

## [0.146.0] - 2026-02-24
### Increase AudioBuffer initial size to 1024 samples

- All modules' `AudioBuffer::new()` changed from 256 to 1024 samples
- Matches cpal backend's actual buffer size and avoids unnecessary reallocation on first audio callback
- Remove unused `Default` impl for `CpalBackend`

## [0.145.0] - 2026-02-21
### Right-click on port → Add new module with auto-connection

**Quick module building directly from ports:**
- Right-click on any port opens a context menu with relevant modules to add
- New module is automatically created and connected to the clicked port
- Menu is filtered based on port type (Audio/Control/Gate) and direction (Input/Output)

**Input ports show source modules:**
- Audio: Oscillator, Sub Osc, Wavetable, Math Osc, Additive, Granular, Noise, Ring Mod
- Control: LFO, Envelope, MSEG, Kinetic Modulator, Envelope Follower
- Gate: Euclidean, Turing Machine, Random Gates

**Output ports show destination modules:**
- Audio: Filter, VCA, Mixer, Signal Monitor + effects (Delay, Reverb, Distortion, Chorus, etc.)
- Control: VCA, Filter, Oscillator
- Gate: Envelope, VCA

**Smart placement:**
- New modules are placed to the left of input ports, to the right of output ports
- Escape or click outside closes the menu

## [0.144.0] - 2026-02-21
### Improved port descriptions with connection suggestions

**Informative tooltips on all ports:**
- All module ports now have descriptions with concrete connection suggestions
- Format: "Short description. Connect: Module1, Module2, Module3"
- Helps new users understand what can be connected where

**Updated modules:**
- Oscillator: FM, PM, PWM, X-Mod, Sync — explains what each input does and suggests modulation sources
- Envelope: Gate, Velocity, Out — with info about automatic connections and destinations
- LFO: Retrig, Rate CV, Out — with suggestions for modulation sources and destinations
- Amplifier (VCA): In, In L/R, CV, Pan CV, outputs — connection suggestions
- Mixer: In 1–8, Out — with suggestions for audio sources
- Filter (SVF + Ladder): In, Cutoff CV, Res CV, Out — suggests Envelope/LFO
- Kinetic Modulator, Sub Osc, Wavetable, Granular, Math Oscillator, Noise, Ring Mod, Signal Monitor, Additive Osc — all with connection suggestions

## [0.143.0] - 2026-02-18
### New — Compact Inline Signal Monitor + Polyphonic sweep lock

**Inline Signal Monitor (100×50px):**
- New compact variant of Signal Monitor inserted via right-click menu on cables
- Shows only oscilloscope waveform without title bar, parameters or controls
- 2 grid cells wide × 1 grid cell tall (100×50px)
- Small port dots on left (in) and right (out) side
- Close button (×) in upper right corner

**Close and reconnect:**
- When closing the inline monitor, cables are automatically reconnected
- Incoming connection (source → monitor) and outgoing (monitor → destination) replaced with direct connection (source → destination)
- Monitor is removed after reconnection

**Polyphonic sweep lock (all Signal Monitors):**
- New `Arc<AtomicBool>` sweep lock shared between all voice clones
- Only one voice at a time writes to the visualization buffer
- "Last triggered voice wins" — new trigger always takes over
- Prevents the messy mixing of multiple voices' waveforms that was displayed before
- Applies to both the large Signal Monitor and the compact inline variant

**Serialization:**
- New PatchModuleType::InlineSignalMonitor for correct save/load
- Inline monitors are preserved on save and restored on load

## [0.142.0] - 2026-02-18
### New — Right-click menu on cables with Signal Monitor insertion

**Cable menu (right-click):**
- Right-click on a cable now opens a context menu instead of directly deleting the cable
- Menu option: "Delete cable" — removes the cable
- Menu option: "Insert Signal Monitor" — inserts a Signal Monitor between the two modules
- Cable glows with its own color (glow effect) on hover instead of bright red

**Automatic Signal Monitor insertion:**
- On insertion, a new Signal Monitor module is created and placed midway between the two connected modules
- Original connection is removed and replaced with: source → Signal Monitor → destination
- Visualization buffer is automatically connected for real-time waveform display

## [0.141.0] - 2026-02-16
### New — Signal Monitor (inline waveform viewer)

**Signal Monitor PolyModule:**
- New voice graph module that can be connected anywhere in the signal chain
- Pass-through: copies input to output without modification
- Displays waveform in real-time with rising-edge trigger detection for stable display
- Parameters: Time (time scale/zoom), Gain (vertical amplification), Trigger (threshold level), Frozen (pause)
- Category: Utility — rendered with in/out ports in three-column layout

**VisualizationSink trait (synth_core):**
- New trait that breaks circular dependency between synth_modules and synth_engine
- SignalMonitor uses `Option<Arc<dyn VisualizationSink>>` with injection from the GUI layer
- VisualizationBuffer implements VisualizationSink in synth_engine

**Improved Oscilloscope:**
- Rising-edge trigger detection for stable waveform display (same algorithm as Signal Monitor)
- Stack-allocated buffers instead of Vec allocations in process() (real-time safe)

**GUI integration:**
- Signal Monitor available in Visualizer palette menu
- Oscilloscope widget with trigger level line (yellow horizontal line)
- Patch serialization (save/load) for Signal Monitor
- Vis buffer cleanup on module removal for Utility/PhysicalModeling categories

## [0.140.0] - 2026-02-16
### New — Orthogonal cables with animated signal flow

**Orthogonal cable routing:**
- Cables drawn as right-angled lines (horizontal→vertical→horizontal) instead of bezier curves
- Visually matches auto-layout's left→right signal flow
- Rounded corners (5px radius) at bends for smoother appearance
- Subtler shadows (1px, 2px offset, alpha 40) fitting the new style
- Nearly horizontal cables (within 8px) simplified to straight lines

**Animated flow particles:**
- Small circles move along cables in signal direction
- Audio: fast particles (120px/s), dense spacing (30px)
- Control/CV: medium speed (60px/s), wider spacing (50px)
- Gate: pulsing particles (80px/s), blinking alpha
- MIDI: 70px/s, 45px spacing

**Improved hit-testing:**
- Exact point-to-line-segment distance instead of bezier sampling
- Faster and more precise hover detection

## [0.139.0] - 2026-02-16
### Improved — Size-aware auto-layout + smart modulation placement

**Modulation modules placed per column:**
- ADSR/LFO placed directly below their target column's signal modules, not below the globally tallest column
- Before: 3 oscillators in column 0 (600px) → ADSR for Filter in column 1 ended up below 600px
- After: ADSR placed directly below Filter (~200px) — saves space and keeps modulators visible

**Mod matrix fix:**
- Fixed slot width (140px) instead of `ui.available_width()` which grew unbounded in auto-sized Areas

## [0.138.0] - 2026-02-16
### Improved — Size-aware auto-layout (no overlaps)

**Problem:** Modules overlapped each other because auto-layout used fixed cell sizes (250×200px) while modules have varying sizes (envelope ~360px tall, oscillator ~260px, LFO ~150px).

**Solution:**
- `ModulePanelState` now saves each module's actual rendered size (`size: Vec2`)
- `patch_editor.rs` updates `panel_state.size` from `area_rect.size()` after each frame
- Auto-layout uses actual sizes instead of fixed cells:
  - Column width = max snapped width of all modules in column + 1 grid cell gap
  - Row positions calculated cumulatively per column with actual (snapped height + gap)
  - Modulation zone starts below the tallest signal column
- `snap_size_to_grid()` rounds up module sizes to whole grid cells (50px)

**New test:** `test_no_overlap_mixed_sizes` — verifies that modules with mixed sizes don't overlap (rect intersection check)

## [0.137.0] - 2026-02-16
### Improved — Auto-layout based on signal flow analysis

**New 5-phase layout algorithm:**
- Phase 1: Classifies modules into four groups — SignalChain, Modulation, Global (Effect/Visualizer/Utility), Disconnected
- Phase 2: Topological depth assignment via Kahn's algorithm → columns left-to-right with longest path
- Phase 3: Vertical ordering within columns with median heuristic (minimizes cable crossings)
- Phase 4: Modulation sources (Envelope/LFO) placed below their primary signal chain targets
- Phase 5: Pixel positions with fixed estimated sizes (ScrollArea handles overflow)

**Improvements compared to previous BFS layout:**
- Parallel signal paths (e.g. two oscillators → mixer) handled correctly
- Output modules forced to last signal column
- Utility modules placed in global zone (not disconnected)
- Cycles handled gracefully via Kahn's algorithm
- Modules no longer overlap each other

**New tests:** `test_multi_source_to_mixer`, `test_complex_patch`, `test_no_overlap`, `test_output_rightmost`, `test_utility_is_global`

## [0.136.0] - 2026-02-16
### Improved — Modules clipped by panels

**Area + Frame instead of Window:**
- Modules now rendered with `egui::Area` + `Frame::window` instead of `egui::Window`
- Modules placed in `Order::Background` (same layer as panels) instead of `Order::Middle`
- Each module Area clipped to scroll area's visible rect via `set_clip_rect(visible_rect)`
- Modules extending outside patch editor are now clipped by surrounding panels

**Manual title bar:**
- Replaces Window's built-in title bar with heading + close button
- Close button shown only for disconnected modules (Disconnected)
- Cables and toolbar overlay still rendered in foreground

## [0.135.0] - 2026-02-16
### Improved — ScrollArea in the patch editor

**ScrollArea with constrain_to:**
- Patch editor content wrapped in `egui::ScrollArea::both()` — scrollbars appear automatically when modules don't fit
- Each module window uses `Window::constrain_to(scroll_rect)` so modules can't be dragged outside the area
- Removed manual canvas panning (`canvas_offset`) — ScrollArea handles scrolling natively
- Grid lines drawn relative to scroll area without offset calculation
- Auto-layout works directly with scroll rect without offset conversion
- Toolbar remains visible in foreground layer, positioned relative to visible area

## [0.134.0] - 2026-02-15
### Improved — AWE perceptually distinct absorption

**Perceptual material values:**
- All 15 materials' absorption values adjusted from physically accurate to perceptually distinct
- Metal/tile/concrete now clearly harder; wood/water/fabric/carpet clearly softer
- Each material has a unique tonal character (e.g. glass: thin in bass, ice: crisp HF absorption)

**sqrt() mapping replaces linear amplification:**
- Removes `ABSORPTION_AMPLIFICATION` (3.0x) from all three DSP files
- sqrt() mapping spreads hard materials better without saturating soft ones
- Widened LP/HP coefficient ranges give larger perceptual differences

**More aggressive room modes feedback:**
- Feedback damping increased from `avg * 0.5` to `avg * 0.8` for clearer material distinction

## [0.133.0] - 2026-02-15
### Fixed - AWE frequency-dependent absorption

**Frequency-dependent damping throughout the entire DSP chain:**
- Replaces `Material::average_absorption()` (a single scalar) with per-band absorption (low/mid/high) through all DSP stages
- Early Reflections: each tap now has separate LP and HP filters instead of a single damping filter
- Room Modes: each comb filter now has LP + HP in the feedback loop
- FDN: `lp_coeff` calculated from `absorption_high`, `hp_coeff` from `absorption_low`
- Absorption Amplification factor (3.0x) to spread small physical differences into audible filter differences
- Different materials (concrete, metal, glass, wood, fabric) now produce distinctly different tonal character

**Fixes in Spatializer (head shadow):**
- Inverted head shadow coefficients: one_pole with coeff 1.0 = full LP (not pass-through)
- Near ear now gets coeff ≈ 0 (pass-through), far ear gets higher coeff (more HF damping)

**Fixes in tests:**
- All awe_engine tests updated with correct newtype wrappers (NormalizedValue, SampleRate, Meters, etc.)
- Presets test fixed with `.as_f32()` conversion

## [0.132.0] - 2026-02-15
### Added - Kinetic Modulator

**Ny modulationsmodul: Kinetic Modulator**
- Gate-triggad kontrollmodul som genererar Position/Velocity/Acceleration via Penner easing-kurvor
- 10 easing-kurvor: Linear, QuadOut, CubicOut, QuartOut, QuintOut, ExpoOut, CircOut, BackOut, ElasticOut, BounceOut
- Alla kurvor med analytiskt beräknade derivator (velocity och acceleration)
- Konfigurerbar duration (0.01–10s), overshoot (Back/Elastic), bipolar-läge
- Loop-lägen: OneShot, Loop, PingPong
- Retrigger-stöd vid ny note-on

**Mod Matrix: 3 nya modulationskällor**
- KineticPos — position från easing-kurvan
- KineticVel — normaliserad hastighet (derivata)
- KineticAcc — normaliserad acceleration (andra derivatan)
- ModSource utökad från 11 till 14 varianter

**Nya patches**
- "Kinetic Pluck" — CubicOut-kurva (150ms) driver filter och amplitud
- "Kinetic Pad" — ElasticOut-kurva (2s) i loop-läge för evolverande padljud

## [0.131.0] - 2026-02-15
### Changed - Isometric 3D AWE view with sound animations

**Isometric 3D rendering (cutaway style):**
- Replaces 2D floor plan with isometric 3D view
- Cutaway style: back wall, right wall and floor visible, front walls omitted
- Solid shading: floor darkest, walls lighter with alpha transparency
- All 6 room shapes rendered isometrically: Box, Cylinder, L-Shape, Sphere, Dome, Tube
- Dimension labels placed along isometric floor edges

**Expanding sound rings:**
- Animated rings expand from source on the floor plane
- New ring every 0.5s, max 6 simultaneous, decreasing in opacity with age
- Rings drawn as isometric ellipses (48 points)
- Reflection rings spawned from mirror sources at walls (Box/Tube)

**Animated reflection lines (marching ants):**
- Dashed reflection lines animated with running offset
- Dashes flow continuously S → wall → L
- Replaces static dashed lines

**Updated interaction:**
- Drag uses inverse isometric projection (screen_to_floor)
- Markers (S/L) placed via iso_to_screen on floor plane
- Spatial mapping dots projected isometrically

## [0.130.0] - 2026-02-15
### Changed - AWE view: Improved graphical representation

**Shape-specific room outline in floor plan:**
- Box: rectangle (as before)
- Cylinder: rectangle with rounded short sides
- L-Shape: L-shaped polygon
- Sphere: circle
- Dome: circle with dashed lower half (hemisphere)
- Tube: rectangle with dashed open short sides

**Reflection paths:**
- First-order reflections shown as dashed lines (source → wall → listener)
- Box: 4 reflections (all walls), Tube: 2 reflections (long sides)
- Calculated with the image source method

**Info box in floor plan:**
- Shows distance (S→L), RT60 (Sabine's formula), and room volume
- Semi-transparent background for readability

**Improved markers:**
- Larger circles (14px) with outline ring
- Arrow from source to listener (triangle arrowhead)
- Hover text "Source" / "Listener" at markers

**Simplified control panel:**
- LFO sections collapsible (closed by default) via CollapsingHeader
- "Impossible" renamed to "Effects" with subtitle "Effects beyond physics"
- Subtitle "Balance between dry/wet signal" under Mix heading
- Tooltips on all parameters: Dry/Wet, Early/Late, Modes, Tail, Freq Warp, Resonance, Portal, Diffusion

## [0.129.0] - 2026-02-15
### Added - AWE indicator, Oscilloscope at piano & AWE newtype migration

**AWE indicator in toolbar:**
- Button shows green dot (●) when AWE effect is active, dimmed/gray when off.
- Still functions as view switcher (AWE/Rack).

**Master output oscilloscope:**
- Left and right channels displayed as oscilloscope next to the piano in the bottom panel.
- Automatically takes the remaining space if the piano doesn't fill the entire width.
- Hidden if the screen is too narrow (< 120px remaining).
- Cyan color for left channel, green for right.

**VisualizationBuffer in EngineState:**
- New `master_scope` buffer in `EngineState` for master output waveform data.
- `SynthEngine` writes final output (after master volume) to the buffer every callback.

### Changed - synth_awe: Complete newtype migration

**New `types` module** with 7 AWE-local newtypes:
- `Meters`, `SquareMeters`, `CubicMeters` — room dimensions and surfaces/volumes.
- `MetersPerSecond` — speed of sound.
- `SampleOffset` — fractional sample position for interpolated delay lines.
- `StretchFactor` — tail stretch (0.5–4.0, clamped).
- `Position3` — 3D position `[Meters; 3]` with `x()/y()/z()` accessors.

**Migration from raw primitives to typed domain values** throughout the crate:
- `f32` → `Meters` (all room dimensions in `RoomShape`, `EarlyReflections`, `RoomModeBank`, `SpatialContext`, `Spatializer`).
- `f32` → `NormalizedValue` (absorption, diffusion, dry/wet, portal amount, LFO amount etc.).
- `f32` → `Gain` (feedback, tap gains).
- `f32` → `FilterState` (one-pole filter states).
- `f32` → `SampleOffset` (delay tap positions).
- `f32` → `SampleRate`, `Seconds`, `Hertz`, `BipolarValue` (all public APIs).
- `f32` → `StretchFactor` (tail stretch parameter).
- `[f32; 3]` → `Position3` (source/listener positions).
- `usize` → `SampleCount` (delay buffer sizes, block sizes).
- `u8` → `MidiNote` (per-voice spatialization).

**DSP improvements (no behavior changes):**
- One-pole filter: manual calculation replaced with `FilterState::one_pole()`.
- Gain application: manual multiplication replaced with `Gain::apply()`.
- Hot-path optimization: filter state and mix parameters hoisted to local variables before per-sample loop.
- Magic numbers replaced with named constants (`PORTAL_MAX_DELAY`, `PORTAL_MAX_DELAY_SAMPLES`).

**All 36 presets** updated with `.into()` conversion.
**GUI (awe_view.rs)** updated with `.as_f32()` extraction and newtype wrapping at the interface with sliders.

## [0.128.0] - 2026-02-15
### Added - AWE: New materials, more presets, diffusion & LFO stability

**9 new materials:**
- Marble, Ice, Carpet, Water, Void, Prism, Plasma, Membrane, Nanogel.
- Creative/non-physical materials (Void, Prism, Plasma, Membrane, Nanogel) for extreme sound design.

**36 presets (up from 14):**
- 22 new presets: including Basalt Chasm, Aurora Hall, Gravity Tunnel, Rain Room, Floating Choir, Mirror Plane, Crystal Vault.
- 6 "EXT:" presets with extreme materials: Singularity, Plasma Storm, Prism Spiral, Membrane Cavity, Nano Fog, Antigrav.
- Existing presets fine-tuned (more realistic dimensions and positions).
- Preset menu split into Standard / Extreme sections.

**Material diffusion in DSP:**
- `Material::diffusion` now affects FDN diffusion (0.35 + diffusion × 0.55).
- Early reflections: per-tap jitter based on diffusion and reduced directional cues.
- Delay buffer extended to 1.0 s (support for rooms up to ~170 m).

**LFO stability (base value tracking):**
- `base_room` and `base_snapshot` save user-set values.
- LFOs reset base values before each modulation pass — eliminates drift.

**Buffer enlargements:**
- FDN: pre-allocation ×48 (from ×32) for large rooms with tail stretch.
- Room modes: max delay 48,000 samples (from 5,000).

**GUI:**
- 15 materials in material selector (from 6).
- Better material matching with multi-band comparison.

**Other:**
- `Instrument::process_visualizers()` called after effect chain.
- Removed `docs/AWE-Implementation-Review.md`.

## [0.127.0] - 2026-02-15
### Added - AWE: New room shapes + Preset menu

**New room shapes (3):**
- **Sphere**: Spherical room (all dimensions = diameter). Coinciding modes give focused resonance.
- **Dome**: Hemisphere (height = radius, width/length = diameter). Dome reflections.
- **Tube**: Open tube without end caps. Less surface area gives longer RT60 and flutter echoes.
- Correct geometry formulas (volume, surface area, axial modes) for all new shapes.
- LFO modulation of RoomLength/RoomWidth works with all 6 shapes.

**AWE Preset menu (14 presets):**
- New preset selector in AWE toolbar with hover descriptions.
- 14 creative presets: Cathedral, Bathroom, Cave, Pipeline, Concert Hall, Sci-Fi Corridor, Dream, Underground, Industrial Hall, Small Studio, Space Station, Mountain Echo, Dome, Portal.
- Presets demonstrate all 6 room shapes, all materials, and Impossible parameters.
- Selecting a preset loads complete AWE state (room, material, mix, LFOs, spatial).
- Manual changes reset preset selection.

**GUI:**
- RoomShapeKind extended with Sphere, Dome, Tube.
- Dimension sliders for all new shapes (radius, length).
- `restore_from()` and `to_awe_state()` handle all 6 shapes correctly.
- Fixed height calculation for source/listener z-position (uses effective room height).

## [0.126.0] - 2026-02-15
### Added - AWE Phase 3: Per-voice Spatialization

**Per-voice room positioning:**
- Each active voice can be assigned its own position in the room based on MIDI note.
- 4 mapping modes: Off, Linear X, Linear Y, Circular.
- Individual early reflections (ISM) per voice with own `EarlyReflections` instances.
- Individual spatializer (ITD/ILD) per voice.
- Shared FDN reverb and room modes fed by summed mono.

**SpatialVoiceBank & SpatialVoicePool:**
- Pre-allocated bank with 16 mono buffers (4096 samples each) - ~1.3 MB total.
- Per-voice DSP pool with 16 slots: `EarlyReflections` (16K delay) + `Spatializer`.
- `NotePositionMapping` enum with `position_for_note()` and `pan_for_note()`.
- `SpatialContext` struct to communicate spatial context to instruments.

**Instrument per-voice capture:**
- `Instrument::process()` now receives `SpatialContext` and `SpatialVoiceBank`.
- Per-voice mono capture to spatial bank in both normal and oversampled paths.
- Per-voice dry panning based on note position relative to listener.

**GUI:**
- New "Spatial" section in AWE control panel with On/Off toggle and Mapping selector.
- Visualization of note positions as faint dots in floor plan.

**Persistence:**
- `spatial_enabled` and `note_mapping` in AweSnapshot, AweState and patch format.
- Backwards-compatible deserialization via `#[serde(default)]`.

**New constructor:**
- `EarlyReflections::with_max_delay()` for customizable delay size.

## [0.125.0] - 2026-02-15
### Added - AWE Phase 2: Advanced Geometry & Creative Features

**New room shapes:**
- Cylinder room (pipeline/tunnel mode) with radius and length.
- L-shaped room (two connected rectangles) with individual dimensions.
- New `RoomShape` variants with volume/surface_area/axial_modes support.
- `DEFAULT_CYLINDER` (r=1m, L=20m) and `DEFAULT_LSHAPE` (8×5 + 6×4, H=3m) constants.

**"Impossible room" parameters:**
- Freq Warp: Modulates FDN LP damping — positive gives more HF reverb.
- Resonance Boost: Adds energy to FDN feedback (clamped at 0.97).
- GUI sliders in new "Impossible" section.

**Acoustic portal:**
- Extra stereo delay path with feedback simulating adjacent virtual room.
- One-pole LP damping for muffled portal sound.
- Portal Amount control (0–1) with smooth ramping.
- `PortalAmount` as new LFO target.

**4 internal LFOs (expanded from 2):**
- LFO 3 & 4 with Rate/Amount/Target controls.
- 13 modulation targets (expanded from 8): +EarlyLate, ModesAmount, ResonanceBoost, TailStretch, PortalAmount.
- Full GUI with 4 LFO sections.

**GUI improvements:**
- Room shape selector (ComboBox: Box/Cylinder/L-Shape).
- Dimension sliders adapted per room shape.
- Floor plan adapted to effective dimensions regardless of shape.
- Slider range extended to 100m for Box (supports "The Void" preset).

## [0.124.0] - 2026-02-15
### Added - AWE Phase 1: Parametric Room

**Early Reflections (ISM):**
- Image Source Method med 6 taps (en per vägg i rektangulärt rum).
- Fraktionella delays via `InterpolatedDelayLine`, per-tap one-pole LP-damping.
- Avståndsberoende gain (1/r), pan baserat på speglad källas X-offset.
- Automatisk geometriuppdatering vid rum-/positionsändringar.

**FDN Late Reverb (geometridrivet):**
- `FdnCore`-baserad sen reverb med parametrar härledda från rumsgeometri.
- RT60 via Sabines formel, feedback-gain beräknad från RT60.
- Delay-tider skalade efter rumsdimensioner, damping från materialabsorption.

**Room Mode Bank (comb-filter):**
- 3 axiella rumsmoder (längd/bredd/höjd) som parallella comb-filter.
- Feedbackstyrka och damping beräknade från absorption.
- Modes amount-kontroll (0–1) för blandning.

**Spatializer (ITD + ILD):**
- Interaural time difference via två `InterpolatedDelayLine` (max 64 samples).
- Interaural level difference via equal-power panning.
- Head shadow one-pole LP per öra, mer dämpning på avskärmade örat.

**AWE-interna LFO:er:**
- 2 kontroll-rate LFO:er (sine, 0.01–20 Hz) som körs per block.
- 8 targets: RoomLength, RoomWidth, SourceX/Y, ListenerX/Y, DryWet, FreqWarp.
- Smooth 5 ms parameter-ramping i DSP-loopen.

**2D Floor Plan GUI:**
- Top-down planritning med rum-outline, dimensionslabels.
- Dragbara källa (S) och lyssnare (L) markörer.
- Material-väljare (6 presets).
- Sliders: Dry/Wet, Early/Late, Modes, Tail Stretch.
- LFO 1 & 2: Rate, Amount, Target-väljare.

**Ny params:**
- `AweLfoTarget` enum och `AweLfoState` struct för LFO-persistence.
- `AweSnapshot` utökad med `lfo1`/`lfo2` fält.
- `AweParam` utökad med Lfo1Rate/Amount/Target, Lfo2Rate/Amount/Target.
- `RoomShape` dimension-accessors: `length()`, `width()`, `height()`.
- `SPEED_OF_SOUND` som publik konstant i `room.rs`.

## [0.123.0] - 2026-02-14
### Added - AWE (Acoustic World Engine) Phase 0 — Infrastructure

**FDN-extraktion (synth_dsp):**
- Ny `fdn`-modul med `FdnCore` struct — extraherad från Reverb.
- 8-kanal FDN med Hadamard-matris, per-kanal damping/lowcut, modulerade delay-linjer.
- `FdnCore::process_sample()` tar mono in, returnerar stereo `FdnStereoOutput`.
- Reverb delegerar nu till `FdnCore` — alla befintliga tester passerar.

**synth_awe crate (ny):**
- `AweEngine`: Pass-through processor (ingen DSP i Fas 0).
- `AweParam`: Enum med RoomShape, Material, SourcePos, ListenerPos, DryWet, m.fl.
- `AweSnapshot`: Copy-struct för batch-uppdatering av numeriska parametrar.
- `AweState`: Serde-serialiserbar state för patch-persistence.
- `RoomShape`: Box-variant med length/width/height, volume(), surface_area(), axial_modes().
- `Material`: 6 konstanter (CONCRETE, WOOD, GLASS, METAL, FABRIC, TILE) med frekvens-beroende absorption.

**Engine-integration:**
- Tre nya `EngineCommand`-varianter: `SetAweParameter`, `SetAweEnabled`, `SetAweState`.
- `AweEngine`-fält i `SynthEngine`, processas efter master effects.
- Master-level visualizers körs nu efter AWE (visar slutsignal).

**GUI:**
- Ny `AppView::AcousticWorld` variant med toggle-knapp i menyraden.
- Placeholder AWE-vy med enable/disable toggle.

**Persistence:**
- `PatchSettings.awe: Option<AweState>` — sparas/laddas automatiskt.

## [0.122.0] - 2026-02-14
### Added - VOSIM, Spectrum Analyzer, Oversampling

**Vosim (MathOscillator):**
- Ny `Vosim` algoritm i MathOscillator — klassisk röstsyntes via kvadrerade sinuspulser.
- 3 parametrar: Formant (1x-20x basfrekvens), Decay (0.3-0.99), Pulser (1-6 st).
- Ger vokalliknande ljud, perfekt för pads och experimentella texturer.

**Spectrum Analyzer (Visualizer):**
- Ny visualizer-modul som visar frekvensspektrum via FFT (2048-punkt).
- Logaritmisk frekvensaxel (20Hz-20kHz) med dB-skala.
- Hann-fönster, mono (L+R)/2 analys, Gain-parameter för vertikal skalning.
- Grid-linjer vid 100Hz, 1kHz, 10kHz + etiketter.
- Full patch save/load-stöd.

**Oversampling (2x/4x):**
- Per-instrument oversampling för minskad aliasing från oscillatorer och waveshapers.
- Ny `OversamplingFactor` (Off/2x/4x) i instrument rack UI.
- 11-tap half-band FIR anti-aliasing filter (~60dB stopband rejection).
- 4x använder kaskaderade 2x-steg (4x → 2x → 1x).
- Pre-allokerade buffertar — noll overhead vid Off (1x), inga heap-allokeringar.
- Röster processas vid högre sample rate, effektkedjan kör vid originalrate.

## [0.121.0] - 2026-02-14
### Added - Polyphonic Aftertouch, Granular Synthesis, Convolution Reverb, Phase Vocoder & FFT Infrastructure

**Polyphonic Aftertouch:**
- New `PolyAftertouch` modulation source in mod matrix (per-note pressure).
- Separate from channel aftertouch — each voice has its own aftertouch value.

**FFT Infrastructure (synth_dsp):**
- New `spectral` module with `realfft`-based utilities.
- `FftProcessor`: Pre-allokerad real FFT wrapper med forward/inverse.
- `StftProcessor`: Overlap-add STFT med ring buffer och spectral callback.
- `PartitionedConvolver`: Uniform partitioned convolution för långa impulse responses.
- `WindowType`: Hann, Hamming, Blackman-Harris fönsterfunktioner.

**Granulär Syntes (GranularOsc):**
- Ny PolyModule med 32 simultana grains (fixed-array, ingen heap i process).
- 5 källvågformer: Saw, Sine, Square, Triangle, Noise.
- 3 fönstertyper: Hann, Gaussian, Trapezoid.
- Parametrar: GrainSize, Density, Position, PositionSpread, PitchSpread, PanSpread.
- Freeze-mode för att låsa grain-position.
- RT-säker xorshift32 PRNG.

**Convolution Reverb (Convolver):**
- Ny AudioEffect med partitioned FFT-convolution (stereo).
- 4 matematiskt genererade impulse responses: Plate, Room, Spring, Hall.
- Pre-delay (0-200ms), Decay Trim, Brightness (one-pole LP) och Mix.
- Automatisk IR-rebuild vid parameterändringar.

**Phase Vocoder:**
- Ny AudioEffect med STFT-baserad pitch shifting (stereo).
- Pitch shift: -24 till +24 halvtoner med fas-ackumulering.
- Spectral freeze-mode (håller nuvarande spektrum).
- Konfigurerbar FFT-storlek: 512, 1024, 2048, 4096.

## [0.120.0] - 2026-02-14
### Added - Cross-Modulation, FDN Reverb, Sidechain & Microtonal Tuning

**Oscillator — Cross-Modulation:**
- Ny `cross_mod` audio-ingång på varje oscillator för bilateral FM mellan oscillatorer.
- `CrossModAmount`-parameter (0.0-1.0) styr modulationsdjup.
- Kombineras med befintlig FM-ingång för komplex frekvensmodulering.

**FDN Reverb (Feedback Delay Network):**
- Helt ny 8-kanals FDN-implementation ersätter tidigare Schroeder-reverb.
- Hadamard-mixningsmatris (8×8) för maximal energispridning.
- Primtalsbaserade delay-tider (2039-3511 samples) skalade med Size-parameter.
- Per-kanal LFO-modulerade delay-tider (~0.3 Hz) för tätare reverb.
- Frekvensberoende dämpning: lowpass (Damping) och highpass (LowCut) per kanal.
- Pre-delay upp till 500ms, Decay- och Diffusion-kontroller.
- Stereo-output med konfigurerbar Width.

**Compressor — Sidechain:**
- Ny `SidechainEnabled`-toggle och `SidechainFilter`-parameter (20-500 Hz HPF).
- `set_sidechain_input()` för extern sidechain-signal som detektionskälla.
- One-pole highpass-filter på sidechain för att isolera transient-detektion.

**Microtonal Tuning:**
- Nytt `TuningTable`-system med [Hertz; 128] MIDI note → frekvens-mappning.
- 5 inbyggda presets: Equal Temperament (12-TET), Just Intonation, Pythagorean, 19-TET, 31-TET.
- Scala-parser: stöd för .scl (skalfiler) och .kbm (keyboard mapping).
- `TuningPreset`-enum med `ALL`, `name()`, `id()`, `from_id()`, `to_table()`.
- Integration i Voice: `set_tuning_table()` ersätter statisk frekvenstabell.

## [0.119.0] - 2026-02-14
### Added - 10 new audio techniques: MSEG, BBD Delay, Additive Synth, Generative modules etc.

**Nya effekter (AudioEffect):**
- **BBD Delay**: Analog bucket-brigade delay-emulering med kompander (tanh), bandbreddsbegränsning, wow & flutter LFO, clock noise och feedback med per-repeat mörkläggning.
- **Brickwall Limiter**: Look-ahead limiter med true peak detection, konfigurerbart ceiling/release, soft knee och gain reduction metering.
- **Mid/Side Processing**: Stereo breddkontroll med M/S encoding, width (0-2x), mid/side gain och mix.

**Nya voice-moduler (PolyModule):**
- **MSEG (Multi-Stage Envelope Generator)**: 16-segments envelope med per-segment time/level/curve, loop-stöd, sustain-segment och tempo-sync. Preset-mallar: ADSR, Tremolo, Sidechain Pump.
- **Additive Oscillator**: 32 harmoniska med spektral profil (Tilt, Odd/Even, Brightness), spektral stretch för inharmonicitet, fas-randomisering vid note-on.
- **Euclidean Sequencer**: Björklund-algoritm för jämnt fördelade pulspar. Steps (1-32), pulses, rotation, swing. Gate + accent CV-output.
- **Turing Machine**: 16-bit skiftregister för semi-slumpmässiga melodier. Mutation rate (locked→evolving→random), skalkvantisering (chromatic/major/minor/pentatonic), pitch CV + gate output.
- **Random Gates**: Probabilistisk gate-generator med density, burst mode, konfigurerbar gate-längd. Gate + random CV output.

**GUI-integration:**
- Alla nya effekter tillgängliga i Effect-menyn och Master Effects-panelen med fullständiga parameter-sliders.
- Additive Oscillator i Oscillator-submenyn.
- MSEG i Modulation-submenyn.
- Ny "Generative"-submeny med Euclidean, Turing Machine och Random Gates.
- Patch-serialisering (save/load) stöds för alla nya modultyper.

## [0.118.0] - 2026-02-14
### Added - 8 nya kreativa patchar med Ring Mod, Envelope Follower & Wavetable
- **Vocal Pad**: Eterisk körliknande pad med Formant-wavetable och LFO-driven vokal-morphing genom vowel-shapes.
- **Metallic Bell**: Skimrande metallisk klocka — Digital wavetable genom Ring Mod med keyboard tracking skapar inharmoniska övertoner.
- **Auto-Wah Bass**: Funky auto-wah bass where Envelope Follower tracks playing dynamics and drives the Acid filter's cutoff in real time.
- **Digital Chime**: Crystalline chime sound with Digital wavetable and envelope-driven position sweep through complex waveforms.
- **Warm Evolving**: Slowly evolving ambient texture with Warm wavetable, very slow LFO scanning and deep reverb.
- **Harmonic Lead**: Expressive lead with Harmonics wavetable — envelope sweeps from simple sine tone to 32 harmonics on each note.
- **Ring Mod Drone**: Deeply evolving drone — LFO modulates Ring Mod carrier frequency for continuously shifting sidebands.
- **PWM E-Piano**: Warm electric piano with PWM wavetable, envelope-driven pulse width sweep, Fluid filter and classic chorus.

## [0.117.0] - 2026-02-14
### Improved - Categorized submenus for Example Patches
- **Categorized patch menu**: Example Patches menu now shows patches in 8 submenus grouped by category instead of a flat list of 35 patches.
- **Categories**: Keys & Piano, Bass, Lead, Pad, Drums, Strings & Bell, Experimental, Ambient & Texture.
- **New function `categorized_patches()`**: Returns patches grouped by category, used by the menu. `example_patches()` remains as a flat list.

## [0.116.0] - 2026-02-13
### Added - Wave 1: Ring Modulation, Envelope Follower, Wavetable Synthesis
- **Ring Modulation**: New voice module that multiplies input signal with internal carrier oscillator. Supports 5 waveforms (sine/tri/saw/square/pulse), keyboard tracking, frequency ratio (0.25x-4.0x), dry/wet mix and freq CV input. Produces metallic bell sounds, sci-fi textures and inharmonic overtones.
- **Envelope Follower**: New voice module that tracks the amplitude of an input signal and produces a control signal (0.0-1.0). One-pole filter with separate attack/release coefficients and sensitivity control. Useful for auto-wah, sidechain-like effects and dynamic modulation.
- **Wavetable Synthesis**: New voice module with 6 built-in wavetable banks:
  - **Basic**: Sine → Triangle → Saw → Square morph (64 frames)
  - **Harmonics**: 1→32 harmonic additive synthesis (32 frames)
  - **PWM**: Pulse width 50%→5% (32 frames)
  - **Formant**: Vocal formants a/e/i/o/u with interpolation (32 frames)
  - **Digital**: FM, hard sync, bitcrush, ring mod-like (32 frames)
  - **Warm**: Soft analog variants with harmonic saturation (32 frames)
- **Wavetable scanning**: Position parameter (0.0-1.0) with CV modulation for timbral movement.
- **GUI integration**: Wavetable in Oscillator menu, Ring Mod and Env Follower in new "Modulation" submenu.
- **Patch serialization**: All three modules support save/load with PatchModuleType.

## [0.115.0] - 2026-02-13
### Added - Character Filters (Analog filter models)
- **FilterModel parameter**: New Model selector in the Filter module with 4 options: Standard, Fluid, Screamer, Acid.
- **Fluid** (Oberheim-inspired): SVF with normalized tanh saturation and continuous Morph control (LP→BP→HP→Notch) via constant-power crossfade.
- **Screamer** (MS-20-inspired): Sallen-Key HP→LP cascade with asymmetric diode clipping in the feedback loop. Aggressive, screaming resonance.
- **Acid** (Steiner-Parker-inspired): ZDF 2-pole with resonance-dependent variable saturation (tanh→sine-fold blend). Supports LP/BP/HP modes.
- **Morph knob**: New parameter for the Fluid model that smoothly crossfades between filter outputs.
- **Real-time safe**: All filter structs are Copy with only f32 fields, no heap allocations.
- **Patch: Fluid Pad**: Evolving pad with the Fluid filter's morph swept by LFO, 5-voice triangle unison and reverb.
- **Patch: Fluid Keys**: Warm electric piano with Fluid morph creating bell-like overtones via envelope sweep, chorus.
- **Patch: Screamer Lead**: Aggressive lead with the Screamer filter's diode clipping, high resonance and fast envelope sweep.
- **Patch: Acid Bass**: Classic 303 acid with the Acid filter's variable saturation, square oscillator and fast cutoff sweep.

## [0.114.0] - 2026-02-13
### Added - Intra-voice Unison in Oscillator
- **Unison voices**: 1-7 detuned oscillator copies inside each voice, producing fat unison sound without multiplying voice cost.
- **Parameters**: Unison (number of voices), Uni Detune (0-100 cent spread), Uni Spread (stereo panning), Uni Phase (phase randomization at note-on).
- **Stereo outputs**: New `out_l` and `out_r` ports with constant-power panning for stereo unison. Mono `out` port is backward-compatible.
- **Real-time safe**: All arrays are fixed-size [T; 7], no heap allocation in process(). Phase randomization via lock-free fastrand.
- **n=1 special case**: When unison is off (1 voice), panning is skipped entirely, exact same behavior as before.
- **Patch: Unison Supersaw**: Classic trance supersaw with 7-voice unison, filter envelope and chorus.
- **Patch: Stereo Unison Pad**: Wide ambient pad with triangle unison via stereo outputs (out_l/out_r) directly to amp.
- **Patch: Unison Sync Lead**: Aggressive hard-sync lead with tight 3-voice mono unison and waveshaper.
- **Patch: Unison PWM Strings**: Lush string ensemble with pulse PWM, 5-voice stereo unison, chorus and reverb.

## [0.113.0] - 2026-02-13
### Added - Waveshaper module
- **Waveshaper effect**: New creative waveshaping module with 6 curves: Soft Clip, Asymmetric, Fold, Chebyshev, Sine Fold, Quantize.
- **Parameters**: Curve (curve selection), Drive (exponential 1x-20x), Mix (dry/wet), Bias (DC offset), Symmetry (asymmetric control).
- **GUI integration**: Available in the effect palette, master bus dropdown, and patch editor.
- **Patch: Waveshaper Lead**: Sharp lead with Sine Fold curve, saw oscillator and filter envelope modulation.
- **Patch: Glitch Pad**: Evolving pad with Fold curve, triangle oscillator and LFO filter modulation.

## [0.112.0] - 2026-02-13
### Changed - Mod Matrix: enabled checkbox & Amount label
- **Enabled checkbox**: Each slot now has a checkbox next to the Amount knob to enable/disable the slot.
- **Shorter knob label**: The knob now shows "Amount" instead of "Slot X Amount".
- **SlotEnabled param**: New `SlotEnabled` parameter per slot controls whether modulation is active.

## [0.111.0] - 2026-02-13
### Fixed - Mod Matrix Grid layout
- **Equal-sized slots**: All slots in the grid now have the same fixed width, calculated from available space and number of columns.
- **Amount label inside frame**: The "Slot X Amount" text below the knob now fits entirely within the group frame border.
- **Dynamic ComboBox width**: Dropdowns adapt their width to the slot size instead of a fixed 80px.

## [0.110.0] - 2026-02-13
### Changed - Mod Matrix Grid redesign
- **Grid-based layout**: Mod Matrix now renders as a grid instead of a flat list. Grid size selected via selectbox: 1x1, 2x2 (default), 3x3, 4x4 — giving 1, 4, 9 or 16 slots.
- **16 slots**: Max number of slots increased from 8 to 16 (4x4 grid).
- **Removed enabled toggle**: Slots with Source=None are automatically inactive, separate on/off toggle removed.
- **Compact cells**: Each cell in the grid shows Source and Destination dropdowns plus Amount knob.
- **Grid size param**: New `GridSize` parameter controls how many slots are processed and displayed.

## [0.109.0] - 2026-02-13
### Changed - Consistent module naming
- **Always number suffix**: All modules now always show instance number (e.g. "LFO 1", "Oscillator 1", "Filter 1") even if there is only one module of that type. Selectboxes now always match the module names in the view.

## [0.108.0] - 2026-02-13
### Improved - Smart module names & filtered Mod Matrix choices
- **Numbered module titles**: When there are 2+ modules of the same type, "Oscillator 1" / "Oscillator 2" is shown. A single module is shown without number ("LFO").
- **Filtered Mod Matrix dropdowns**: Sources and destinations referencing modules that don't exist in the patch are hidden. E.g. "LFO 2" is not shown as source if only one LFO exists, "Osc 2 Pitch" is hidden without a second oscillator.
- **PatchAnalysis**: New internal analysis struct that counts module types per frame and drives both naming and filtering.

## [0.107.0] - 2026-02-13
### Added - Mod Matrix (8-slot modulation routing)
- **Mod Matrix module**: New 8-slot modulation routing system that lives in each voice
- **10 modulation sources**: None, LFO 1/2, Env 1/2, Velocity, Note Number, Aftertouch, Mod Wheel, Pitch Bend
- **11 modulation destinations**: None, Osc 1/2 Pitch, Osc 1 Level, Filter 1/2 Cutoff, Filter 1 Reso, Amp Level/Pan, LFO 1 Rate/Depth
- **Bipolar amount**: Each slot has -1.0 to +1.0 scaling with knob control
- **Enable/disable per slot**: Toggle to enable/disable individual slots
- **Voice integration**: Modulation is applied automatically before graph processing, with one-block latency (~1ms)
- **Mod offsets in destination modules**: Filter, oscillator, amplifier and LFO now support modulation offsets
- **GUI**: Mod Matrix available via "Mod Matrix" button in the module palette (Utility category)
- **Patch serialization**: Mod Matrix settings are saved and loaded with patches

## [0.106.0] - 2026-02-13
### Changed - Keyboard shows more keys when window is wider
- **Fixed key sizes**: White keys 24px, black keys 14px — size never changes
- **More keys at wider width**: Wider window shows more of the 88 keys, centered when all fit
- **Conditional scroll**: Scroll and scroll indicators are only shown when all keys don't fit

## [0.105.0] - 2026-02-13
### Changed - Resizable module windows
- **Resizable module windows**: Modules can now be dragged wider/narrower with `resizable(true)`. Center content fills available width, OUT ports always sit against the right edge.

## [0.104.0] - 2026-02-13
### Fixed - Compact module windows
- **Auto-fit height**: Replaced `StripBuilder::horizontal` (which expanded cells to full available height) with `ui.horizontal` + `ui.vertical` — module windows now adapt height to their content without dead space
- **Fixed module width**: Modules no longer expand with the main window — content column uses `set_min_width` instead of `available_width()`
- **Non-resizable windows**: Module windows are now `resizable(false)` and always auto-fit to content
- **Removed `StripBuilder` dependency**: `egui_extras::StripBuilder` and `Size` are no longer used in module layout

## [0.103.0] - 2026-02-13
### Changed - StripBuilder for module layout
- **StripBuilder layout**: Replaced manual `ui.horizontal()` + gap-fill with `egui_extras::StripBuilder` for the three-column layout (IN | content | OUT). Port columns use `Size::exact()` and center content uses `Size::remainder()`, giving exact column widths without fragile gap calculation.
- **Right column flush**: OUT ports now sit guaranteed flush against the module's right edge thanks to StripBuilder's fixed column sizes.
- **Simplified `draw_port_column`**: Removed manual `set_min_width`/`set_max_width` — StripBuilder handles column width.

## [0.102.0] - 2026-02-11
### Changed - New module layout: ports on the sides
- **Three-column layout**: Ports are now rendered in vertical columns to the left (IN) and right (OUT) of the module content, instead of in a horizontal section between header and parameters. Reduces module height and eliminates dead space.
- **Port columns**: 28px wide columns with ports centered vertically, labels shown as tooltips on hover
- **Effect/Visualizer modules**: Keep full width without port columns (have no ports)
- **Increased minimum module width**: 140px → 180px (28px port column + 100px content + 28px port column + margins)
- **New theme constants**: `port_column_width`, `port_vertical_spacing`, `module_content_min_width` in `Sizes`
- **Auto-layout updated**: `MIN_MODULE_WIDTH` increased to 180px

## [0.101.0] - 2026-02-11
### Fixed - 3 GUI↔Engine bugs
- **Cable disconnection (CRITICAL)**: `connections_to_remove` from patch editor was never processed — Disconnect commands are now sent to engine
- **Bypass for voice modules (HIGH)**: `SetBypass` only searched in effect chain. Bypass is now supported in `ModuleGraph` (voice graph) with zeroed outputs for bypassed modules
- **SetTempo (LOW)**: `EngineCommand::SetTempo` was caught by catch-all `_ => {}` — now connected to `TransportState::set_tempo()`
- Removed unreachable catch-all (`_ => {}`) in command matching now that all variants are handled

## [0.100.0] - 2026-02-11
### Removed - Dead code and non-functional GUI elements
- **Mixer view**: The entire placeholder view (8 dummy faders + "coming soon" text), `AppView::Mixer` variant, view selector in top bar
- **layout.rs**: Unused alternative top bar implementation (290 lines), `TopPanel` enum
- **Sample dialog**: `OpenSample` variant, `open_sample_dialog()` method, match arm in file dialog handling (never called, handler was TODO)
- **"Audio settings coming soon"**: Placeholder section in the Settings dialog
- **MIDI Refresh button**: Button in MIDI dropdown that only closed the menu without doing anything
- **MultiPointEnvelope**: Tracker-specific envelope types (`MultiPointEnvelope`, `EnvelopePoint`, `EnvelopeType`)
- **Tracker references**: Remaining dead code and imports related to removed tracker functionality
- **Sample code**: `SamplePlayer` pitch mod, `WaveformOverview`, `PlaybackPositionBuffer`, hound reference in ARCHITECTURE.md

## [0.99.0] - 2026-02-10
### Removed - All tracker import functionality
- **Decision**: After v0.87–v0.98 (2 days, 9 bug fixes) we realized that tracker playback (XM/MOD/S3M via xmrs) doesn't fit the architecture. The synth engine is polyphonic/semitone-based while tracker requires period-based pitch with tightly coupled effect processing. Each fix revealed new bugs.
- **Removed**: Tracker import (`TrackerImporter`), tracker effect processing (`tracker_effects.rs`, ~2200 lines), tracker patterns (`tracker_pattern.rs`, ~670 lines), tracker effect types (`effects.rs`), tracker views, tracker-specific fields in Voice/SynthEngine/Song, Sequencer GUI view, all tracker analysis examples (9 total), all tracker reference documents
- **Kept**: Basic `SequencerEngine` (simple NoteOn/NoteOff playback for piano patterns), `synth_sequencer` crate with Pattern/Song/Note types
- **Tagged**: `v0.98.0-tracker-experiment` — last version with tracker code
- **See**: `docs/tracker-experiment-summary.md` for full analysis and future alternatives

## [0.98.0] - 2026-02-09
### Fixed - Vibrato/arpeggio applied incorrectly at tick 0
- **Problem**: `ChannelEffectState.current_tick` was never reset at row start. The field retained the value from the previous row's last `process_tick()` (e.g. tick 4 at speed=5). When `current_modulation()` was called at tick 0, `current_tick.as_u8() > 0` was true, causing vibrato to be applied — even though FT2 uses `realPeriod` (zero vibrato offset) at tick 0.
- **Consequence**: Every row with vibrato had an incorrect pitch offset at tick 0. With vibrato depth=1, the offset could be up to ±12.45ct (about 1/8 semitone) instead of 0ct. In FT2, tick 0 functions as an "anchor point" where pitch returns to the base note, but our code had a random vibrato offset (depending on phase from the previous row's last tick). With 113 vibrato effects in a single pattern, this sounded consistently "out of tune".
- **Fix**: Sets `state.current_tick = TickInRow::ZERO` in `process_row_start()` immediately after the channel's state is fetched. The vibrato check `current_tick > 0` in `current_modulation()` now correctly evaluates to false at tick 0.
- **Verification**: Debug output now shows `pitch=+0.00ct` at tick 0 for all vibrato rows (confirmed with `debug_playback`).
- **Impact**: All XM/MOD/S3M files with vibrato (4xx), vibrato+volslide (6xx), or arpeggio effects. Arpeggio tick cycle (tick % 3) was also incorrect at tick 0.

## [0.97.0] - 2026-02-09
### Fixed - TonePortamento 4x too slow in Amiga mode
- **Problem**: `apply_amiga_tone_portamento()` divided `tone_porta_speed` by 4.0 before using it as a period step. The comment claimed "FT2 Amiga tone portamento uses raw_param", but the FT2 source code shows that `portaSpeed = param << 2` (multiplies by 4 at setup) and then uses `portaSpeed` directly in `tonePorta()` — exactly the same as regular portamento.
- **Consequence**: Tone portamento (3xx/5xx) ran 4x slower than in FT2. With param=16 (effect 310) and speed=5, FT2 gives 64 period units/tick x 4 ticks = 256 periods/row = 4 semitones. Our code gave 16 period units/tick x 4 ticks = 64 periods/row = 1 semitone. Tone slides never reached their targets in time, resulting in audibly out-of-tune notes.
- **Fix**: Removed the `/4.0` division in `apply_amiga_tone_portamento()`. `tone_porta_speed` (= raw_param x 4.0) is now used directly as period step, identical to `apply_amiga_portamento()`.
- **Verification**: Comparison with FT2-clone source code (`ft2_replayer.c`: `tonePorta()` + `getNewNote()`) confirms that `portaSpeed` is used without division.
- **Impact**: All XM/MOD/S3M files with Amiga frequency mode and TonePortamento effects (3xx, 5xx). 567 TonePortamento effects in the test file `joli_untouched.xm`.

## [0.96.0] - 2026-02-09
### Fixed - Extra effect tick per row (25% too much portamento/vibrato/volume slide)
- **Problem**: `process_tick()` was called 5 times per row instead of 4 at speed=5. In FT2 with speed=5 there are 5 ticks (0-4): tick 0 is handled by `process_row_start()`, ticks 1-4 are handled by `process_tick()`. But our engine called `process_tick()` every 40th song tick, and with 200 song ticks per row (5x40) that became 200/40 = 5 calls instead of 4.
- **Consequence**: All continuous effects got **25% more effect per row** (5/4 = 1.25x):
  - PortamentoDown(5): -156.25ct/row instead of -125.00ct/row
  - Vibrato: phase advanced 25% faster → faster and wider vibrato
  - Volume slide: volume change 25% faster
  - Tone portamento: glide reached target 25% faster
  - **Over 2 rows, portamento drift became -312.50ct instead of -250.00ct — a difference of 62.5ct (~0.6 semitones), clearly audible "out of tune notes".**
- **Fix**: In `process_tick()`, if `tick_in_row >= speed`, current modulation is returned without applying effects. The 5th process_tick iteration (tick_in_row=5 at speed=5) is now correctly skipped.
- **Impact**: All XM files with continuous effects (portamento, vibrato, tremolo, volume slide, panning slide).

## [0.95.0] - 2026-02-09
### Fixed - pitch_offset leaks from PortamentoUp to TonePortamento
- **Problem**: `apply_amiga_portamento()` (1xx/2xx) accumulated pitch change in `pitch_offset`, while `apply_amiga_tone_portamento()` (3xx) worked on `current_pitch`. In FT2 there is only one period variable, but our code has two separate ones (`current_pitch` + `pitch_offset`). When TonePortamento followed after PortamentoUp, the accumulated `pitch_offset` was not absorbed, causing it to be added on top of TonePortamento's result.
- **Consequence**: With PortamentoUp(2) over 5 ticks, +62.5ct accumulated in `pitch_offset`. When TonePortamento then slid `current_pitch` toward the target note (e.g. E-6 = 88.0), the final pitch ended up at 88.0 + 0.625 = 88.625 semitones — **62.5 cents too high**. Vibrato then oscillated around this incorrect pitch.
- **Fix**: In `process_row_start`, when TonePortamento is detected, existing `pitch_offset` is absorbed into `current_pitch` and `pitch_offset` is reset to zero. TonePortamento then slides from the correct start position.
- **Impact**: All patterns where PortamentoUp/Down (1xx/2xx) is followed by TonePortamento (3xx).

## [0.94.0] - 2026-02-09
### Fixed - Tone Portamento target delayed by one row
- **Problem**: In the XM import (`process_track_unit_to_cell`), `last_porta_target` was updated AFTER effects were processed. TonePortamento (3xx) thus received the previous row's note as target instead of the current row's note.
- **Consequence**: In the playback code, `trigger_note` correctly set `tone_porta_target` to the current note, but then the effect processing overwrote it with the incorrect (old) target value. The portamento slide always started one row too late, and the first row with portamento produced no pitch change at all.
- **Fix**: Pitch is now calculated and `last_porta_target` is updated BEFORE the effect loop runs, so that TonePortamento always gets the correct target note.
- **Impact**: All channels with TonePortamento effects (3xx) in imported XM/MOD/S3M files.

## [0.93.0] - 2026-02-09
### Enhanced - Extended Debug button in Sequencer view
- **Channel mute status**: Shows active/MUTED status for each track
- **Instrument defaults**: Shows volume and panning per instrument (matches `analyze_tracker` format)
- **Full pattern grid**: Prints the entire pattern content in the same format as the `analyze_tracker` example (notes, instruments, volume, effects)
- **Effect summary**: Counts and lists all used effects in active pattern sorted by frequency
- Debug output is now directly comparable with `cargo run --example analyze_tracker`

## [0.92.0] - 2026-02-07
### Fixed - Tremolo, vibrato and volume slide accuracy

#### Tremolo depth ~4x too weak
- **Problem**: `TremoloDepth::from_param` used `depth / 64.0` but the FT2 formula is `(waveform_peak * depth) >> 6` where peak=255. Result: tremolo was ~4x too weak and nearly inaudible.
- **Fix**: New formula `depth * 255.0 / 64.0 / 64.0` matches FT2's depth scale.

#### Vibrato offset applied at tick 0
- **Problem**: Vibrato offset was calculated and applied on all ticks including tick 0. FT2 resets vibrato offset at tick 0 (`outPeriod = realPeriod`) and doesn't run `doVibrato` until tick 1+.
- **Fix**: Vibrato offset is now only applied on ticks 1+. Vibrato/tremolo phase is also only advanced on ticks 1+.

#### Volume slide priority rule incorrect
- **Problem**: `SlideRate::from_volume_slide` subtracted `up - down`, but FT2 gives the upper nibble (UP) priority when both are non-zero. Same bug in `from_panning_slide`.
- **Fix**: New priority logic — if `up > 0`, `down` is ignored (and vice versa for panning slide).

### Added - Effect accuracy analysis
- New file `docs/effect-accuracy-analysis.md` — thorough analysis of all tracker effects against FT2 reference
- New file `docs/references/ft2-effect-reference.md` — complete FT2 effect reference with exact formulas and C code from ft2-clone

## [0.91.0] - 2026-02-07
### Fixed - Continuous effects leak between rows

#### Continuous effects (volume slide, portamento, vibrato, etc.) not stopped between rows
- **Problem**: Effects like volume slide, portamento, vibrato, tremolo and panning slide continued running on rows where they were not specified. State was never reset — only updated WHEN the effect was present. Result: effects "leaked" and ran indefinitely until a new note with fresh attack was triggered.
- **Root cause**: `process_row_start()` processed effects in a match loop but never reset continuous state before the loop. XM effect memory (param=0 = "continue") was confused with "effect is active".
- **Fix**: Continuous effect state is now reset BEFORE the effect loop each row. Effect memory is saved in local variables and only restored WHEN the effect actually exists on the row (with param=0). New `tone_porta_active` field distinguishes "active this row" from "has memory value".

#### Affected effects
- **Volume slide (Axy)**: Stopped on rows without Axy
- **Portamento up/down (1xx/2xx)**: Stopped on rows without 1xx/2xx
- **Tone portamento (3xx)**: Stopped on rows without 3xx/5xx (new `tone_porta_active` field)
- **Vibrato (4xy)**: Depth reset on rows without 4xy (phase preserved)
- **Tremolo (7xy)**: Depth reset on rows without 7xy
- **Panning slide (Pxy)**: Stopped on rows without Pxy
- **Fine volume slide (EAx/EBx)**: Stopped on rows without EAx/EBx
- **Arpeggio (0xy)**: Stopped on rows without 0xy
- **Retrigger (E9x)**: Stopped on rows without E9x
- **Note cut/delay/fadeout**: Cleared each row

#### Known limitation
- XM effect 5xy (TonePortamento+VolumeSlide) and 6xy (Vibrato+VolumeSlide) with param 0 are not handled fully correctly by the xmrs library — it drops VolumeSlide(0,0). With this fix, volume slide is stopped correctly, but 500/600 "continue both" only continues the first sub-effect.

## [0.90.0] - 2026-02-07
### Fixed - Portamento conversion in Amiga mode (and linear mode)

#### Portamento ~41x too fast and inverted direction (CRITICAL)
- **Problem**: Portamento effects (1xx/2xx) glide ~41x too fast and in the wrong direction. Three bugs interact:
  1. Import multiplies with `* 16.0` instead of dividing with `/ 4.0` to restore raw param
  2. Import inverts direction (xmrs negative = porta UP, not DOWN)
  3. Effect processor converts speed incorrectly with `/ 100.0 * 64.0` instead of using directly
- **Fix**: Import now restores raw param correctly (`speed.abs() / 4.0`), direction fixed, effect processor uses period units directly in Amiga mode

#### Tone portamento incorrect scaling
- **Problem**: Tone portamento (3xx) is scaled incorrectly in both import and effect processor
- **Fix**: Import now handles Amiga/Linear separately. Effect processor converts correctly: Amiga `/4.0`, Linear `*1200.0/768.0/100.0`

#### Linear portamento incorrect conversion
- **Problem**: Linear portamento adds period units directly as cents, without conversion
- **Fix**: Now converts period units to cents with `* 1200.0 / 768.0`

#### Debug tool (analyze_tracker_raw)
- Fixed direction and scaling in `format_track_effect` for Portamento and TonePortamento

## [0.89.0] - 2026-02-07
### Fixed - XM Speed Effect Silence & GUI Row Sync

#### Silence on dynamic speed change (CRITICAL)
- **Problem**: XM modules that change speed with Fxx effect (e.g. F03 = speed 3) caused silence after half the pattern. Pattern placements were calculated at import with default speed (6, 240 ticks/row), but dynamic speed 3 (120 ticks/row) processed all rows twice as fast — halfway through the tick window all rows were done and the rest became silent.
- **Fix**: New method `auto_advance_if_past_pattern()` in the sequencer engine that detects when all rows have been processed and jumps directly to the next pattern. Notes are preserved across pattern boundaries (no release).

#### GUI tracker row out of sync on speed change
- **Problem**: The sequencer view used static `ticks_per_row` (240) to calculate which row is displayed. With dynamic speed 3, the GUI ran at half speed and broke at half the pattern.
- **Fix**: Dynamic `ticks_per_row` is now shared from the audio thread to the GUI via `TransportState` (atomic). The GUI calculates the row with `offset / dynamic_ticks_per_row` instead of the pattern's static value.

## [0.88.0] - 2026-02-07
### Changed - Solo/Mute per channel & Module name in toolbar

#### Solo/Mute buttons in Sequencer
- **Solo (S):** No longer toggleable — click mutes all other channels and unmutes the selected one
- **Mute (M):** New button per channel — toggleable individual mute with red/gray indication
- **"Unmute All" button** in the toolbar for quickly removing all mutes
- Replaced `solo_track: Option<TrackId>` with `muted_tracks: Vec<bool>` throughout the stack (state, engine, commands)
- Multiple channels can now be unmuted simultaneously (not limited to a single solo channel)

#### Module name in toolbar
- Module name (song.name) is now displayed in the sequencer toolbar after the Debug button

#### Solo/Mute in Rack view
- The solo button in the instrument rack now works like in the sequencer: click mutes all other instruments
- New **"Unmute All" button** in the instrument rack header
- Removed toggle-based solo state (`InstrumentUiState::solo` is no longer used for solo toggle)

## [0.87.0] - 2026-02-06
### Fixed - XM Playback & Tracker Display Improvements

#### Tracker velocity fix (KRITISK)
- **Problem**: `Velocity::FF` (112/127 = 0.882) användes för tracker NoteOn istället för `Velocity::MAX` (1.0), vilket gav 12% volymreduktion
- **Fix**: Ändrat till `Velocity::MAX` i `sequencer_engine.rs` för tracker-läge

#### Vibrato fasbevarande (FT2-kompatibilitet)
- **Problem**: Vibrato-fas återställdes till noll vid varje ny not, men FT2 bevarar fasen mellan noter
- **Fix**: Borttagen `vibrato_phase = Phase::ZERO` från `trigger_note()` i `tracker_effects.rs`

#### GUI tracker display
- **Fix**: `SetVolume`-effekten separeras nu till volymkolumnen istället för att visas som "C08" i effektkolumnen
- Borttagen redundant effektkolumn — XM har bara 1 effektkolumn, `fasttracker()` config ändrad från 2→1
- Volymkolumnen visar nu bara hex-värde (t.ex. `08`) utan "C"-prefix

#### Analysverktyg
- Ny `analyze_tracker_raw.rs` — visar rå xmrs-representation (PatternSlot, TrackUnit)
- Omskriven `analyze_tracker.rs` — visar intern representation (Cell, EffectCommand)
- Ny `debug_playback.rs` — tick-för-tick uppspelningslogg
- Raw-analyzern separerar nu Volume-effekter till volymkolumnen (matchar intern representation)
- Regenererade alla debug-filer i `docs/debug/`

#### CLAUDE.md
- Dokumenterat debug- och analysverktyg, debug-output, GUI debug-knapp
- Instruktion att nya tekniska referenser ska sparas i `docs/references/` med uppdaterad README.md

## [0.86.0] - 2026-02-06
### Added - Sequencer Debug Button & Tracker Analysis Column Fix

#### Debug-knapp i sequencer toolbar
- Ny "Debug"-knapp i sequencerns toolbar (till höger om Pattern-navigering)
- Skriver ut song-info, aktiv pattern-data och arrangement till konsolen
- Visar: song-namn, BPM, speed, frequency mode, antal tracks/patterns/instruments
- Aktiv pattern: namn, rader, kanaler, ticks per row, antal noter/noteoffs/effekter
- Arrangement: alla pattern-placeringar med pattern-ID, track-ID och tick-position
- Matchar format från `analyze_tracker.rs` för enkel jämförelse

#### Fixad kolumnformatering i analyze_tracker
- Effektkolumnen paddas nu till 4 tecken (`{:<4}`) i `format_cell()`
- Löser inkonsekvent bredd: 3-teckens effektkoder (t.ex. `F70`) vs 4-teckens tom (`....`)
- Alla celler har nu konsekvent 14 teckens bredd, kolumner ligger rakt

## [0.85.0] - 2026-02-06
### Fixed - XM Playback Bugs (Double Volume, Fine Volume Slide Memory, Amiga Portamento)

Tre buggar som påverkade XM-uppspelningskvaliteten (identifierade via joli_suspiria.xm och joli_untouched.xm):

#### Bugg 1: Dubbel volymapplicering (KRITISK)
- **Problem**: Sample `default_volume` multiplicerades in BÅDE via `SamplePlayer.level` OCH via tracker_volume kanal-modulation → `output × vol²` istället för `output × vol`
- **Fix**: Borttagen `.with_default_volume()` från tracker sample-import (`tracker.rs`). Volym hanteras enbart via `TrackerInstrumentDefaults` → channel modulation
- **Effekt**: Korrekt volymbalans, inga onaturligt tysta instrument

#### Bugg 2: FineVolumeSlide saknade effektminne (MEDEL)
- **Problem**: `EA0`/`EB0` (fine volume slide med param=0) gjorde ingenting, borde återanvända senaste slide-värde
- **Fix**: Nytt fält `fine_volume_slide: SlideRate` i `ChannelEffectState` med effektminne — param 0 fortsätter med föregående värde
- **Effekt**: Gradvisa fade-in/out i untouched.xm fungerar korrekt (1586 fine volume slides)

#### Bugg 3: Amiga portamento i linjärt rum (MEDEL)
- **Problem**: XM-filer med `AmigaFrequencies` körde portamento i cents-rum (linjärt), men Amiga-mode ska använda period-rum (hyperboliskt)
- **Fix**:
  - Ny enum `TrackerFrequencyMode { Linear, Amiga }` i `synth_sequencer::song`
  - Lagras i `Song.tracker_frequency_mode`, sätts vid import
  - `ChannelEffectProcessor.amiga_mode` vidarebefordras från `SequencerEngine`
  - Period-baserad portamento: `period = 7680 - (semitones - 12) * 64`, ger karaktäristisk icke-linjär pitch-kurva
  - Påverkar PortamentoUp/Down och TonePortamento

## [0.84.0] - 2026-02-06
### Improved - Enhanced Tracker Analysis Tool & History Date Corrections

#### analyze_tracker.rs — Komplett omskrivning
- **Full pattern grid dump**: Tracker-stil format med NOT IN VL EFCT-kolumner per kanal, hex radnummer
- **Panning envelope**: Visar panning envelope-punkter, sustain, loop (pitch envelope finns ej i xmrs)
- **Full pattern order**: Visar hela pattern-ordningen utan trunkering
- **Helper-funktioner**: `format_note()`, `format_cell()`, `format_track_effect()`, `format_global_effect()`
- **Effects summary**: Räknar alla effekttyper över alla patterns, sorterat efter frekvens
- **Extra debug-info**: Key-off-räkning, keymap-intervall för multi-sample instrument, sample format/finetune/panning, frekvenstyp

#### docs/history.md — Datumkorrigeringar
- Ersatt 130+ "2025" årsangivelser med faktiska YYYY-MM-DD datum från git-historiken
- 8 versioner utan git-commits (0.64, 0.65, 0.47, 0.49, 0.33.13, 0.33.17, 0.24, 0.13.2) fick interpolerade datum

## [0.83.0] - 2026-02-06
### Fixed - Envelope Bugfixes for XM/S3M/MOD Playback

9 buggar identifierade i `docs/instrument-envelope-analysis.md` åtgärdade. Envelope-rendering följer nu FT2-specifikationen betydligt bättre.

#### Prio 1 — Felaktig rendering (påverkade ljud direkt)
- **Linjär fadeout istället för exponentiell**: `FadeoutRate::to_linear_fade_per_tick()` — FT2 använder linjär subtraktion (`fadeoutVol -= speed`), inte multiplikativ decay. Fadeout med rate 4096 når nu tystnad på 8 ticks (0.16s) istället för 107 ticks (2.14s).
- **Parallell fadeout + envelope**: `MultiPointEnvelope` kör nu fadeout parallellt med envelope-avancering efter release, precis som FT2. Borttagna `Fadeout` och `Releasing` stages, ersatta med `released`-flagga.
- **FT2 sustain/loop-interaktion**: Om `sustain_point == loop_end` och noten har släppts, stoppas loopen och envelopet fortsätter förbi loop end.

#### Prio 2 — Saknad funktionalitet
- **Panning envelope**: Extraherar nu `pan_envelope` från xmrs, skapar en andra `MultiPointEnvelope` med bipolar output (-1.0 till +1.0) kopplad till Amplifier `pan_cv`.
- **Gate envelope för instrument utan volume envelope**: MOD/S3M-instrument (inga envelopes) får nu en minimal ADSR-gate (A=0.001, D=0.001, S=1.0, R=0.005) så att NoteOff kan tysta loopad sample.
- **Dynamisk tick_rate från BPM**: `MultiPointEnvelope` läser nu `context.tempo` i `process()` och uppdaterar tick_rate automatiskt. Initial tick_rate sätts från songens BPM vid import.

#### Prio 3 — Förbättringar
- **ADSR release-beräkning fixad**: `convert_envelope_to_adsr()` använder nu korrekt formel `32768 / fadeout / tick_rate` istället för `1 / fadeout` — release-tider stämmer nu med FT2.
- **Korrekt fadeout-divisor**: Linjär fadeout normaliserar mot 32768 (FT2-standard), inte 65536.

#### Filer ändrade
- `synth_core/src/types/tracker.rs`: `to_linear_fade_per_tick()`, uppdaterad `estimated_duration()`
- `synth_modules/src/multi_point_envelope.rs`: Omdesignad stage-maskin, parallell fadeout, bipolar output, dynamisk tick_rate
- `modular_synth/src/io/import/mod.rs`: Panning envelope-fält i `ImportedInstrument`
- `modular_synth/src/io/import/tracker.rs`: Extraherar panning envelope, fixad ADSR release
- `modular_synth/src/gui/egui_backend.rs`: Panning envelope-modul, gate envelope, tick_rate setup

## [0.82.0] - 2026-02-06
### Improved - Tick-Segmented Chunk Rendering for Tracker Playback

Tracker-mode (XM/MOD) renderar nu ljud i tick-segmenterade chunks istället för att applicera alla events vid buffer-start. Detta ger korrekt per-tick modulationsupplösning för vibrato, volume slides, arpeggio, tremolo och andra tick-baserade effekter.

#### Ändringar
- **`SequencerEngine::process_until_next_tick()`**: Ny publik metod som processar exakt ett tick-segment och returnerar chunk-storlek i samples. Befintlig `process()` refaktorerad att anropa den internt — identiskt beteende för alla anropare.
- **`SequencerEngine::is_tracker_mode()`**: Ny metod som detekterar tracker-songs (baserat på `tracker_pattern_count > 0`). Sätts automatiskt vid `set_song()` och `with_song()`.
- **`route_sequencer_events()`**: Extraherad fristående funktion för event-routing (NoteOn, NoteOff, Modulation, VoiceOff). Används av båda render-patherna — ingen kodduplicering.
- **Tick-segmenterad render-loop**: I tracker-mode renderas varje tick-chunk separat med korrekt modulation state. Synth-mode-pathen är helt oförändrad.
- **`chunk_buffer`**: Pre-allokerad AudioBuffer för chunk-rendering, ingen heap-allokering i audio thread.

## [0.81.0] - 2026-02-05
### Fixed - Synth Modules Review Verified Bugfixes

Alla kvarvarande buggar från den verifierade granskningen (docs/synth_modules_review_verified.md) är åtgärdade.

#### Prio 1 — Buggar som påverkade ljud
- **Oscillator prev_sync**: `prev_sync` var lokal variabel i `process()` — retriggrade vid varje buffergräns om sync hölls hög. Flyttad till struct-fält.
- **LFO prev_retrigger**: Identiskt problem — `prev_retrigger` lokal i `process()`. Flyttad till struct-fält.
- **MathOscillator FM base frequency**: FM-modulering hardkodade 440 Hz istället för aktuell basfrekvens. Lagt till `base_frequency`-fält som sätts vid `note_on()` och `set_param()`.
- **MultiPointEnvelope VelocitySensitivity**: Parametern skrev över velocity direkt istället för att kontrollera sensitivity. Nytt `velocity_sensitivity`-fält med korrekt formel: `1.0 - sensitivity * (1.0 - velocity)`.

#### Prio 2 — Vilseledande UI/parametrar
- **Filter env_amount**: Borttagen från descriptor och get_params() — parametern visades i UI men påverkade inte ljudet (ingen envelope-input port finns).
- **SamplePlayer Pan**: Exponerad i descriptor och get_params() — panning fungerade internt men kunde inte ses eller justeras i UI.
- **Compressor sidechain**: Port borttagen från descriptor — var deklarerad men aldrig läst i process().

#### Prio 3 — Kodkvalitet & Newtypes
- **GlideState newtypes**: Alla fält konverterade till domäntyper (`Hertz`, `Seconds`, `NormalizedValue`). Visibility sänkt till `pub(crate)`.
- **Envelope/MultiPointEnvelope trigger()**: Tar nu `Velocity` istället för rå `f32`.
- **MechanicalNoise envelope_phase**: Oanvänt fält borttaget.

#### Prio 4 — Ljud/prestanda-förbättringar
- **Distortion foldback**: While-loop ersatt med for-loop (max 16 iterationer) + final clamp för realtidssäkerhet.
- **Phaser stereo offset**: Höger kanal har nu 90° LFO-fasförskjutning — kommentaren sade "offset phase for stereo width" men båda kanalerna använde samma koefficient.
- **Reverb dirty flag**: `update_filters()` körs nu bara vid parameterändringar (via `params_dirty`-flagga), inte varje buffer.
- **EQ sample_rate**: Jämförelse ändrad från `abs() > 1.0` till korrekt `!=` equality.

#### Verifiering
- Alla ändringar passerar `RUSTFLAGS="-D warnings" cargo build`, clippy (strict), 132 tester och fmt.

## [0.80.0] - 2026-02-05
### Fixed - XM/MOD Playback Accuracy

Omfattande buggfixar för tracker-uppspelning, verifierade mot BassoonTracker och FT2-specifikation.

#### Effekt-buggar
- **Tick-0 modulering saknas**: `process_row_start()` emitterade ingen Modulation-event för tick 0 — stale volym/panning kunde kvarstå efter ny not. Fixat med `emit_tick0_modulation()` helper.
- **is_significant() filtrerade bort nödvändiga moduleringar**: Volym 1.0 och center panning betraktades som "insignifikanta" — tick-0 moduleringar emitteras nu alltid utan filter.
- **XM Arpeggio-ordning**: ProTracker-ordning (0,x,y) → FT2-ordning (0,y,x).
- **PatternDelay (EEx) inte implementerat**: `GlobalCommand::PatternDelay` var no-op. Nu fryser raden N extra ticks med `pattern_delay_rows_remaining`.
- **EffectWaveform::Random deterministisk**: `sample(phase)` returnerade samma hash för samma fas. Nu använder xorshift32 per tick (FT2-beteende).
- **Vibrato depth scaling**: `* 4.0` → `* 2.0` för korrekt FT2-skalning (~30 cents max vid depth 15).
- **Tremolo modell**: Ändrad från multiplikativ (`volume *= 1+tremolo`) till additiv (`volume += tremolo`) per tracker-standard.

#### Sample Player-buggar
- **LoopMode::Backward fungerade inte**: `note_on()` satte alltid `direction = Forward`. Nu startar backward-loopar från loop_end med backward-riktning.
- **PingPong-overshoot förloras**: PingPong satte position till `loop_end - 1.0` utan overshoot-beräkning. Nu bevaras overshoot vid riktningsbyte.
- **Sample offset vid initial NoteOn**: Sample offset-effekt appliceras nu vid första noten.

#### Tracker Engine
- **PatternBreak timing**: Korrekt hantering av pattern breaks vid radgränser.
- **Loop precision**: Exakta loop-punkter (u32) istället för normaliserade f32-värden.

#### Verifiering
- Jämförd med BassoonTracker (JavaScript tracker player) — sample import, loop-hantering och effektprocessing verifierade.
- Alla ändringar passerar `RUSTFLAGS="-D warnings" cargo build`, clippy, tester och fmt.

## [0.79.0] - 2026-01-29
### Fixed - Synth Modules Review Issues

Fixar baserade på omfattande kodgranskning av synth_modules (se docs/synth_modules_review.md).

#### Kritiska buggar
- **Envelope/MultiPointEnvelope gate edge detection**: `prev_gate` var lokal variabel → retriggrades vid buffer-gräns. Fixat genom att flytta till struct-fält.
- **SamplePlayer ReleaseMode::PlayToLoop**: Beter sig nu korrekt - stannar vid loop-slut istället för sample-slut.
- **MechanicalNoise divide-by-zero**: `envelope_samples` kunde bli 0 → division by zero. Fixat med `.max(1)`.

#### Oanvända parametrar fixade
- **Oscillator phase_offset**: Nu applicerad i `generate_sample()` för statisk fasoffset.
- **LFO retrigger_mode**: Nu respekterad - `Continue` ignorerar retrigger input, `Retrigger` aktiverar den.
- **Flanger/Phaser rate_cv**: Port borttagen - AudioEffect trait stöder inte named CV inputs.
- **Chorus Voices parameter**: Nu exponerad i descriptor.

#### Oanvända parametrar implementerade
- **Distortion BitDepth**: Nu exponerad i descriptor och använd i bitcrush-läge.
- **Mixer Input5-8**: Nivåparametrar för alla 8 ingångar nu tillgängliga.
- **Mixer LimitMode**: Soft limiting (tanh) implementerat i process().
- **Filter Drive**: Nu exponerad i descriptor och applicerad som pre-gain med soft saturation.

#### Rust best practices
- `#[must_use]` tillagt på `SampleValue` och `SampleIndex`.
- `velocity: f32` → `Velocity` i VoiceAllocator, Instrument, EngineCommand och EngineEvent.

#### Ändringar
- `Envelope.prev_gate: f32` - nytt fält för edge detection
- `MultiPointEnvelope.prev_gate: f32` - nytt fält
- `SamplePlayer.advance_position()` - stöd för `PlayToLoop`
- `MechanicalNoise.trigger()` - guard mot envelope_samples=0
- `Oscillator.generate_sample()` - adderar phase_offset till fas
- `Lfo.process()` - respekterar retrigger_mode
- Flanger/Phaser descriptor - rate_cv port borttagen
- Chorus descriptor - Voices parameter tillagd
- `DistortionParam::BitDepth` - nytt enum-variant
- `MixerParam::Input5-8` - nya enum-varianter
- `Mixer.process()` - soft limiting via tanh()
- `Filter.process_sample()` - drive pre-gain med saturation
- Velocity typ-säkerhet genom hela note-flödet

---

## [0.77.0] - 2026-01-29
### Fixed - Tracker Effect Issues

Ytterligare fixar för korrekt tracker-uppspelning baserat på djupgående kodgranskning.

#### Sample offset (9xx) vid initial NoteOn
- **Problem**: 9xx-effekten applicerades endast vid retrigger, inte vid första note-start
- **Orsak**: NoteOn-eventet saknade sample_offset fält, offset kom endast via Modulation
- **Fix**:
  - Lagt till `sample_offset: NormalizedValue` i `SequencerEvent::NoteOn`
  - `process_row_start()` returnerar nu även sample_offset
  - `synth_engine.rs` applicerar offset direkt efter `note_on_fixed_voice()` via `retrigger_with_offset()`

#### PatternBreak (Dxx) timing korrigerad
- **Problem**: PatternBreak väntade på pattern-slut istället för rad-slut
- **Orsak**: Villkoret var `current_tick >= current_end` (pattern end) istället för `at_row_boundary`
- **Fix**: PatternBreak triggas nu vid rad-gräns, precis som PatternJump (Bxx)
- Resultat: Dxx hoppar nu omedelbart vid rad-slut, inte vid pattern-slut

#### Loop-precision mismatch åtgärdad
- **Problem**: Olika loop-punkter i `advance_position()` vs `read_with_crossfade()` orsakade klick
- **Orsak**: `advance_position()` använde f32-normaliserade värden som förlorade precision
- **Fix**: Nya fält i SamplePlayer:
  - `exact_loop_start: Option<usize>` - exakt loop-start från sample.loop_info
  - `exact_loop_end: Option<usize>` - exakt loop-slut från sample.loop_info
  - `advance_position()` använder nu dessa exakta värden när de finns

#### Tekniska ändringar
- `SequencerEvent::NoteOn.sample_offset: NormalizedValue` - nytt fält
- `ChannelEffectProcessor::process_row_start()` returnerar nu `(Vec<GlobalCommand>, bool, NormalizedValue)`
- `SamplePlayer.exact_loop_start/exact_loop_end` - exakta loop-bounds
- PatternBreak använder nu `at_row_boundary` istället för `current_tick >= current_end`

---

## [0.76.0] - 2026-01-29
### Fixed - Tracker Effect Accuracy

Omfattande fix av 6 kritiska tracker-effektproblem som orsakade felaktig uppspelning av XM/MOD-filer.

#### SetSpeed (Fxx) påverkar row-timing dynamiskt
- **Problem**: SetSpeed-effekten ändrade bara tempo, inte ticks per rad
- **Orsak**: `ticks_per_row` beräknades en gång vid start, uppdaterades aldrig
- **Fix**: Nytt fält `current_ticks_per_row` som uppdateras vid varje SetSpeed-kommando
- `collect_tracker_pattern_events()` använder nu den dynamiska ticks_per_row

#### Pattern navigation timing (Bxx/Dxx/E6x)
- **Problem**: PatternBreak/Jump/Loop triggades på godtyckliga tick-positioner
- **Orsak**: Navigation skedde direkt istället för vid rad-gränser
- **Fix**: Nya fält `pending_pattern_loop_back` och `last_row_index`
- Navigation väntar nu på rad-gränser innan den appliceras
- Loop-state rensas korrekt vid pattern-byte

#### NoteDelay (EDx) fungerar korrekt
- **Problem**: Noten triggades direkt vid rad-start, ignorerade delay
- **Orsak**: `trigger_note()` anropade `note_state.trigger()` ovillkorligt
- **Fix**: Ny `has_note_delay` parameter i `trigger_note()`
- Noten triggas inte direkt om EDx finns - sker i `process_tick()` vid rätt tick

#### Sample offset (9xx) beräkning korrigerad
- **Problem**: Offset-värdet var fel - delades med 256 istället för att användas direkt
- **Orsak**: xmrs ger oss `parameter * 256`, men importen antog råvärdet
- **Fix**: `let raw_param = ((*offset) / 256).min(255) as u16;` i tracker.rs
- Nu ger 980 (param 0x80 = 128) korrekt 50% av samplelängden

#### Instrumentbyte kapar föregående röst
- **Problem**: Vid instrumentbyte fortsatte gamla instrumentet att spela
- **Orsak**: Endast nya instrumentet triggades, gamla tystades inte
- **Fix**: Vid NoteOn i tracker-läge anropas nu `voice.reset()` på alla andra instruments voice på samma kanal
- Förhindrar överlappande ljud vid instrumentbyte

#### Multisample-instrument med keymap
- **Problem**: XM-instrument med flera samples ignorerade keymap
- **Orsak**: Endast första samplet laddades, keymap aldrig användes
- **Fix**: Ny sample bank-arkitektur i SamplePlayer:
  - `sample_bank: Vec<Arc<Sample>>` - alla samples för instrumentet
  - `sample_keymap: Option<Vec<usize>>` - MIDI-not till sample-index
  - `select_sample_for_note()` - väljer rätt sample vid note_on
- Nytt `LoadSampleBank` kommando för att ladda flera samples med keymap
- Import-koden laddar nu alla samples och keymappen

#### Tekniska ändringar
- `SequencerEngine.current_ticks_per_row: u32` - dynamisk speed
- `SequencerEngine.pending_pattern_loop_back: bool` - rad-boundary navigation
- `SequencerEngine.last_row_index: Option<u32>` - spårar senaste rad
- `ChannelEffectState.trigger_note()` - ny `has_note_delay` parameter
- `SamplePlayer.sample_bank: Vec<Arc<Sample>>` - multisample stöd
- `SamplePlayer.sample_keymap: Option<Vec<usize>>` - not-till-sample mappning
- `SamplePlayer::select_sample_for_note()` - väljer sample från bank
- `SamplePlayer::add_sample_to_bank()` - lägger till sample i bank
- `SamplePlayer::set_sample_keymap()` - sätter keymap
- `PolyModule::load_sample_bank()` - ny trait-metod
- `EngineCommand::LoadSampleBank` - nytt kommando
- `Graph::load_sample_bank()` - ny metod

---

## [0.75.0] - 2026-01-23
### Fixed - Critical Code Issues

Löser 5 kritiska problem identifierade i kodanalysen.

#### Zero-allocation Mixer Hot Path
- **Problem**: `format!("in{i}")` allokerade minne 8 gånger per audio frame i Mixer
- **Orsak**: Dynamiska portnamn krävde String-allokering
- **Fix**: Nya statiska konstanter `PortName::IN1..IN8` och `PortName::MIXER_INPUTS`
- Interned strings i synth_core: "in1" genom "in8" (ID 23-30)
- Mixer använder nu `inputs.get(PortName::MIXER_INPUTS[idx])` - noll allokering

#### Exponential Backoff i send_blocking()
- **Problem**: 10 sekunder timeout (10000 × 1ms) kunde frysa GUI
- **Orsak**: Konstant 1ms sleep oavsett köläge
- **Fix**: Exponential backoff `[0, 0, 1, 2, 4, 8, 16, 32, 64, 100]` ms
- Max 50 försök × ~10ms = ~500ms worst-case (istället för 10s)
- Initiala försök använder `yield_now()` för minimal latens

#### Säker .expect() hantering i GUI
- **Problem**: 5× `.expect()` på instrument-lookup kunde panica vid race conditions
- **Orsak**: `active_instrument_id` kunde bli osynkad vid deletion/creation
- **Fix**: `active_patch_editor()` returnerar nu `Option<&mut PatchEditor>`
- Alla 12 anropsställen uppdaterade med `let Some(editor) = ... else { return; }`
- GUI visar "No active instrument" istället för panic

#### Audio Error Tracking
- **Problem**: Audio stream errors syntes bara på stderr, ej i GUI
- **Fix**: `CpalStream` har nu `error_count: Arc<AtomicU64>` och `last_error`
- Fel loggas till stderr OCH sparas för framtida GUI-display
- Infrastruktur redo för statusfält/notifikationer

#### Realtime-Safe Effect Processing
- **Problem**: Delay/Reverb/Chorus/Flanger anropade `resize_buffers()` i `process()`
- **Orsak**: Buffer-resize kan allokera minne i audio thread
- **Fix**: Ny `set_sample_rate()` implementation för varje effekt
- `process()` anropar inte längre resize - bara `debug_assert!` för verifiering
- Bufferallokering sker nu enbart från main thread

#### Tekniska ändringar
- `PortName::IN1..IN8` - kompilerade konstanter för mixer-portar
- `PortName::MIXER_INPUTS: [PortName; 8]` - iteration utan allokering
- `CommandSender::send_blocking()` - exponential backoff
- `SynthApp::active_patch_editor()` - returnerar `Option`
- `CpalStream.error_count/last_error` - error tracking
- `Delay/Reverb/Chorus/Flanger::set_sample_rate()` - buffer-resize

---

## [0.74.0] - 2026-01-23
### Fixed - Sample Loop Click Prevention

Eliminerar klick och hack vid loop-punkter i importerade tracker-moduler.

#### Exakta loop-punkter (ingen precisionsförlust)
- **Problem**: Loop-punkter konverterades från exakta u32 till normaliserade f32
- **Orsak**: Precisionsförlust på flera samples för stora samplar (>100k frames)
- **Fix**: `SampleLoopInfo` lagrar nu `loop_start` och `loop_end` som `u32`
- Nya metoder `normalized_start()` och `normalized_end()` för GUI-kompatibilitet
- xmrs loop-punkter (u32) bevaras exakt genom hela kedjan

#### Loop-medveten interpolation
- **Problem**: Interpolation (Cubic, Hermite, Sinc etc.) läste samples utanför loop-regionen
- **Orsak**: `clamp(0, len-1)` använde sample-gränser, inte loop-gränser
- **Fix**: Ny metod `Sample::read_looped()` med loop-aware sample-hämtning
- Vid loop_end wrapar interpolation tillbaka till loop_start
- Alla 7 interpolationslägen har nu loop-medvetna varianter

#### Förbättrad crossfade
- **Fix**: `read_with_crossfade()` använder nu `read_looped()` konsekvent
- Crossfade-regionen läser också med korrekt loop-wrapping
- Föredragna loop-gränser från sample-metadata (exakta heltal)

#### Tekniska ändringar
- `SampleLoopInfo.loop_start: u32` - exakt sample-position (ej f32)
- `SampleLoopInfo.loop_end: u32` - exakt sample-position (ej f32)
- `SampleLoopInfo::from_normalized()` - bakåtkompatibilitet
- `SampleLoopInfo::normalized_start/end()` - för GUI
- `Sample::read_looped()` - loop-aware interpolation
- Loop-aware versioner av: cubic, hermite, lagrange, sinc8, sinc16
- `get_looped_sample_mono/stereo()` - sample-hämtning med loop-wrapping

---

## [0.73.0] - 2026-01-23
### Fixed - Tracker Module Playback

Grundlig analys och fix av XM/MOD-moduluppspelning som hade flera kritiska problem.

#### Tone Portamento (3xx/5xx) fungerade inte
- **Problem**: Tone portamento-pitch ignorerades helt i synth_engine
- **Fix**: `tracker_tone_porta_pitch` fält i Voice som override:ar base pitch
- **Fix**: Modulation-events extraherar nu `tone_porta_pitch` och applicerar det
- Glide-effekter fungerar nu korrekt istället för diskreta hopp

#### Initial tracker speed ignorerades vid import
- **Problem**: XM-modulers `default_tempo` (speed) lästes inte vid import
- **Fix**: Nytt fält `default_tracker_speed` i Song-strukturen
- **Fix**: SequencerEngine läser och applicerar speed vid `with_song()` och `set_song()`
- Moduler spelas nu i korrekt tempo från start

#### Tone Portamento triggade felaktigt nya noter
- **Problem**: Noter med tone portamento-effekt skapade NoteOn-events
- **Fix**: `process_row_start()` returnerar nu `(Vec<GlobalCommand>, bool)`
- **Fix**: NoteOn skapas endast om `should_trigger_note` är true
- Förhindrar sample-retrigger vid glide, bevarar envelope

#### Stop-knappen fungerar nu för tracker-moduler
- **Problem**: Voices fortsatte spela efter stop - krävde panic för att tysta
- **Orsak**: `sequencer.stop()` returnerade VoiceOff-events men de ignorerades
- **Fix**: `EngineCommand::Stop` anropar nu `all_notes_off()` på alla instrument direkt
- Voices tystnar nu korrekt vid stop

#### Tekniska ändringar
- `Voice.tracker_tone_porta_pitch: Option<Semitones>` - tone portamento pitch override
- `Song.default_tracker_speed: u8` - initial speed (ticks per row)
- `ChannelEffectProcessor.process_row_start()` - utökad returtyp
- `SequencerEngine.stop()` - skickar VoiceOff för alla MonoVoice-tracks
- Pitch beräknas korrekt: `440.0 * 2^((semitones - 69) / 12)`

---

## [0.72.0] - 2026-01-09
### Improved - Global Module Handling

#### Globala moduler utan portar
- **Effektmoduler** (Reverb, Delay, Chorus, Distortion): Portarna borttagna
- **Visualizer-moduler** (Oscilloscope, LevelMeter): Portarna borttagna
- Tydliggör att globala moduler processas automatiskt via effect chain
- Informativ text visas istället för portar: "Processed automatically via effect chain"

#### Förbättrad visuell feedback för globala moduler
- **Opacity**: Globala moduler dimmas inte längre (alltid full opacity)
- **Ny indikator**: Blå diamant (◆) med tooltip "⚡ Global Module"
- Tydligt att dessa moduler alltid är aktiva

#### Auto-layout förbättringar
- Globala moduler placeras nu längst till höger i rack-vyn
- Egen kolumn efter signalkedjan, före bortkopplade moduler
- Bättre visuell separation mellan signalflöde och globala effekter

#### Patch-filer korrigerade
- 13 exempelpatchar återställda från felaktig effekt-routing
- Effekter hanteras via effect chain, inte via manuella kablar

---

## [0.71.0] - 2026-01-09
### Improved - Rack GUI UX

#### Port-highlighting vid kabeldragning
- **Nytt**: När man drar en kabel lyser kompatibla portar upp med en glödande effekt
- Porten glöder i sin egen färg (Audio=grön, Control=blå, Gate=gul, MIDI=lila)
- Visar tydligt vilka portar som går att koppla till (rätt typ + rätt riktning)
- Implementerat i `PortWidget` med ny `highlighted()` builder-metod

#### Större portar för bättre precision
- **Port-storlek**: Ökad från 14px till 20px för lättare klickning
- **Port-etiketter**: Ökad från 9px till 11px för bättre läsbarhet
- Förbättrar UX på högupplösta skärmar och för pekskärmar

#### Zoom-funktionalitet borttagen
- Tog bort icke-fungerande zoom i Rack-vyn
- Renare kod utan trasig funktionalitet

---

## [0.70.0] - 2026-01-09
### Improved - Module Stability & Performance

#### Envelope (ADSR) förbättringar
- Ersatte `NormalizedValue::new_unchecked()` med säker `new()` + `.clamp(0.0, 1.0)`
- Förhindrar potentiella out-of-bounds värden vid extrema förhållanden

#### Filter stabilitet
- **SVF Filter**: Resonans clampad till max 0.99 för att förhindra instabilitet vid självoscillation
- **Ladder Filter**: Samma resonans-begränsning för stabilitet
- **Denormal prevention**: Lade till `flush_denormals()` för konsekvent prestanda i båda filtertyper

#### Oscillator anti-aliasing
- Lade till PolyBLAMP-korrektion för triangelvågor vid fasens hörnpunkter (0.25 och 0.75)
- Ny `poly_blamp()` funktion i synth_dsp för band-limited triangle waves

#### LFO beat-synkronisering
- LFO-fasen beräknas nu från `context.position_beats` när tempo sync är aktivt
- Perfekt taktlåsning istället för fri löpande frekvens

#### Amplifier (VCA) ny funktion
- Ny parameter `CV Bipolar` för att tillåta negativ CV-modulation
- Möjliggör ring modulation-effekter via VCA

#### DSP primitiver
- **FilterState**: Ny `flush_denormals()` metod förhindrar prestandaproblem med denormala tal
- **InterpolatedDelayLine**: Ny `read_cubic()` metod med Hermite-interpolation för högre kvalitet

#### Body Resonance
- Dynamisk Nyquist-begränsning baserat på sample rate
- Denormal prevention i filter states

#### Sample Player optimering
- Skippar dyra `powf()` per-sample beräkningar när ingen pitch modulation används
- Märkbar CPU-besparing vid normal uppspelning

---

## [0.69.0] - 2026-01-09
### Refactoring - Clippy `use_self` Compliance

#### Systematisk kodstädning
- **Omfattning**: ~150 `use_self` varningar fixade i hela kodbasen
- **Syfte**: Följer Rust best practice att använda `Self` istället för typnamn i impl-block

#### Påverkade crates
- **synth_core** (12 fixar): amplitude.rs, audio.rs, frequency.rs, normalized.rs, samples.rs, module_traits.rs
- **synth_modules** (1 fix): sample_player.rs
- **synth_sequencer** (64 fixar): time.rs, track.rs, pitch.rs, view/state.rs
- **synth_engine** (~70 fixar): transactions.rs, connectivity.rs, event_priority.rs, visual_state.rs, commands.rs, hub.rs, graph.rs
- **modular_synth** (2 fixar): midi.rs, debug_pattern.rs

#### Övriga förbättringar
- Fixade `collapsible_if` lint i debug_pattern.rs
- Alla kontroller passerar: build, clippy, test, fmt

---

## [0.68.0] - 2026-01-09
### Fixed - CPAL 0.17 API Compatibility

#### Uppdaterad cpal-backend för cpal 0.17
- **Problem**: Kompileringsfel efter cpal 0.17 uppgradering
- **Orsak**: cpal 0.17 ändrade flera API:er:
  - `SampleRate` är nu en type alias för `u32`, inte en tuple struct
  - `min_sample_rate()`, `max_sample_rate()`, `sample_rate()` returnerar nu `u32` direkt
  - `Device::name()` är deprecated

#### Ändringar i cpal_backend.rs
- Tog bort `cpal::SampleRate(...)` konstruktor, använder `u32` direkt
- Tog bort `.0` access på sample rate-returvärden
- Bytte från `device.name()` till `device.description().name()` för att undvika deprecation-varningar

---

## [0.67.0] - 2025-12-27
### Fixed - XM Effect Memory & Arpeggio Tick

#### Effect Memory implementerad
- **Problem**: `A00` (volume slide), `100`/`200` (portamento), `P00` (panning slide) nollställde effekten istället för att fortsätta med föregående värde
- **Orsak**: XM-format använder parameter 0 för "fortsätt med föregående hastighet", men koden skrev alltid över effekt-state
- **Lösning**: Lade till villkor `if param > 0` för:
  - `VolumeSlide { up, down }` - behåller slide vid `up=0, down=0`
  - `PortamentoUp(speed)` - behåller speed vid `speed=0`
  - `PortamentoDown(speed)` - behåller speed vid `speed=0`
  - `PanningSlide { left, right }` - behåller slide vid `left=0, right=0`

#### Arpeggio tick-fix
- **Problem**: Arpeggio (0xy) använde `vibrato_phase` för att cykla noter, vilket gjorde att arpeggio inte fungerade när vibrato var av
- **Lösning**:
  - Nytt fält `current_tick: TickInRow` i `ChannelEffectState`
  - `process_tick()` sparar nu aktuell tick
  - Arpeggio använder `self.current_tick.as_u8() % 3` för notcykling

#### Påverkade låtar
- `joli_suspiria.xm` position 3+ låter nu korrekt (massvis av `A00` kommandon)

---

## [0.66.0] - 2025-12-23
### Fixed - XM Vibrato/Tremolo Speed Import

#### Korrigerad effect-skalning
- **Problem**: Vibrato/tremolo visades som `412` istället för `462` i tracker-vyn
- **Orsak**: xmrs normaliserar vibrato/tremolo speed med divisor 64, inte 16
- **Lösning**: Ändrade multiplikator från `* 16.0` till `* 64.0` för speed-parametern

#### xmrs Parameter Memory
- Beslutat att förlita sig på xmrs's inbyggda "parameter memory"
- Continuation effects (`400`) visas nu som resolved values (`462`)
- Förenklar import-koden och följer xmrs design

#### Kod-städning
- Fixade clippy `double_must_use` varningar i `tracker_effects.rs`
- Tog bort redundanta `#[must_use]` attribut på konstruktorer som returnerar Self
- Nytt debug-verktyg: `dump_patterns.rs` example för XM pattern-analys

---

## [0.65.0] - 2025-12-22
### Feature - Extended Tracker Navigation

#### Pattern-navigering under uppspelning
- `<`/`>` knappar fungerar nu även under uppspelning
- Sequencern söker till det nya patterns startposition

#### Play Pattern-funktion
- Ny knapp (🔁) i tracker toolbar
- Spelar endast aktivt pattern i loop
- Nytt `EngineCommand::PlayPattern`

#### Resume Song
- Play-knappen (▶) startar nu från aktivt patterns början
- Istället för att alltid starta från tick 0
- Nytt `EngineCommand::PlayFromPattern`

#### Nya EngineCommands
- `Seek { tick }` - Hoppa till specifik tick-position
- `PlayPattern { pattern_id }` - Loopa ett pattern
- `PlayFromPattern { pattern_id }` - Starta från patterns början

---

## [0.64.0] - 2025-12-22
### Documentation - ARCHITECTURE.md

#### Ny arkitekturdokumentation
- Skapad `ARCHITECTURE.md` för AI-assisterad utveckling
- Dokumenterar crate-struktur och beroendekedja
- Beskriver dataflöde mellan UI och audio-tråd
- Definierar nyckelkoncept (Voice, Instrument, ModuleGraph, etc.)
- Listar kritiska invarianter (realtidssäkerhet, newtype-mönster)
- Inkluderar vanliga operationer (lägga till modul, effekt)

---

## [0.63.0] - 2025-12-22
### Refactoring - Newtype Pattern för synth_engine

#### tracker_effects.rs - Beskrivande typer
- **`RetriggerState`** - Kapslar in retrigger-logik (interval, counter, volume_change)
- **`TickCount`** - Newtype för tick-räkning
- **`Glissando`** enum - `Smooth` / `Quantized` ersätter `bool`
- **`NoteState`** - Ersätter tre booleans med:
  - `PlayingState` enum: `Stopped`, `Playing`
  - `TriggerAction` enum: `None`, `Trigger`
  - `CutAction` enum: `None`, `Cut`

#### effect_chain.rs - Slot-tillstånd
- **`EnabledState`** enum - `Active` / `Bypassed` ersätter `enabled: bool`
- Metoder: `is_active()`, `is_bypassed()`, `toggle()`

#### connectivity.rs - Port och fel-typer
- **`ConnectionState`** enum - `Disconnected` / `Connected`
- **`SignalActivity`** enum - `Inactive` / `Active`
- **`ConnectionCount`** newtype - Ersätter `usize`
- **`SampleTimestamp`** newtype - Ersätter `timestamp: u64` (med serde)
- **`OccurrenceCount`** newtype - Ersätter `occurrence_count: u32`

#### instrument.rs - Mute/Solo med synth_core typer
- Använder **`MuteState`** från synth_core istället för `enabled: bool`
- Använder **`SoloState`** från synth_core istället för `solo: bool`
- Nya metoder: `mute_state()`, `set_mute_state()`, `solo_state()`, `set_solo_state()`

#### Förbättrad typsäkerhet
- Följer CLAUDE.md regler för newtype-mönstret
- Eliminerar primitiver i publika API:er
- Alla 87 tester passerar

---

## [0.62.0] - 2025-12-21
### Fixed - Volume Reset Bug in Tracker Effects

#### Bugfix: Kanalvolym återställs vid ny not med instrument
- **Problem**: Efter volume slide down (t.ex. `A05`) gick kanalvolymen till 0, och nya noter med instrument förblev tysta
- **Orsak**: `process_row_start()` i effektprocessorn fick aldrig information om instrumentbyte, så volymen återställdes aldrig
- **Lösning**:
  - Ny parameter `instrument_volume: Option<NormalizedValue>` i `process_row_start()`
  - När en not med explicit instrument spelas, återställs kanalvolymen till notens velocity
  - Volume slide nollställs för att stoppa pågående slide
  - SetVolume-effekter kan fortfarande override:a default-volymen

#### XM-beteende implementerat
| Scenario | Beteende |
|----------|----------|
| Not + explicit instrument | Återställ volym till notens velocity |
| Not + ärvt instrument (inherit) | Behåll nuvarande kanalvolym |
| SetVolume på samma rad | Override:ar default-volymen |

#### Nya tester
- `test_volume_reset_on_new_instrument` - Verifierar volymåterställning
- `test_volume_not_reset_on_inherit_instrument` - Verifierar att inherit behåller volym
- `test_set_volume_overrides_instrument_default` - Verifierar SetVolume override

---

## [0.61.0] - 2025-12-18
### Added - Track Solo & UI Improvements

#### Track Solo Button
- Ny solo-knapp per track i tracker-vyn
- Klicka "S" för att isolera en track (endast den tracken spelar)
- Klicka igen för att stänga av solo
- Orange bakgrund när aktiv, grå när inaktiv
- Filtrerar NoteOn/Modulation events baserat på voice_index/track

#### UI-förbättringar
- **Instrumentval-knapp**: Nu synlig som radio-knapp (● fylld / ○ tom)
- **Solo-knapp**: Tydlig toggle-stil med synlig bakgrund

#### Borttaget: TrackerFilter
- Tog bort oanvänd `TrackerFilter` modul
- XM-formatet stödjer inte Zxx filter-effekter (endast IT-format)
- Relaterade typer `TrackerCutoff` och `TrackerResonance` borttagna

#### Optimering: Tomma instrument
- Instrument utan giltiga samples skapas nu utan moduler
- Sparar minne och CPU för tomma instrument-platser i XM-filer
- Indexering bevaras för korrekt MIDI-kanal-routing

---

## [0.60.0] - 2025-12-18
### Fixed - SamplePlayer Loop & Keyboard Routing

#### SamplePlayer loop fix
- **Problem**: Loop returnerade till fel position (stannade vid slutet istället för loop_start)
- **Orsak**: När `loop_end == sample_len`, klampas position till `sample_len-1`, vilket gav negativt overshoot i loop-beräkningen
- **Lösning**: Om overshoot är negativt (triggered by clamping), hoppa direkt till `loop_start`

#### Keyboard routing fix
- **Problem**: Flera instrument spelade samtidigt vid tangentbordsinput efter XM-import
- **Orsak**: Tangentbordet skickade noter på instrumentets MIDI-kanal, men `focused_instrument` filtrerade bara på kanal 0
- **Lösning**: Tangentbord skickar alltid på CH1, `focused_instrument` styr routing

#### Sequencer routing
- Sekvensern spelar nu alla instrument oavsett `focused_instrument`-inställning
- `focused_instrument` påverkar endast tangentbordsinput, inte sekvenser-uppspelning

---

## [0.59.0] - 2025-12-17
### Fixed - Tracker Effects (Vibrato, Portamento, Volume Slide)

Fixade två buggar som förhindrade tracker-effekter från att höras:

#### Problem 1: process_tick() anropades aldrig
- `ChannelEffectProcessor::process_tick()` beräknade effekter men resultaten ignorerades
- **Lösning**: Anropa `process_tick()` varje tick och emittera `Modulation` events

#### Problem 2: Steppy pitch modulation
- `SamplePlayer::effective_speed()` beräknades en gång per buffer (~5.3ms)
- **Lösning**: Lade till `pitch_mod` CV-input för per-sample pitch modulation

#### Nya typer och events
- `SequencerEvent::Modulation` - Per-tick modulation från tracker-effekter
  - `pitch_cents: Cents` - Pitch offset (100 cents = 1 halvton)
  - `volume: NormalizedValue` - Volymmodulation
  - `panning: BipolarValue` - Panoreringsmodulation
  - `note_cut: bool` - Note cut flag
  - `tone_porta_pitch: Option<f32>` - Tone portamento target

#### Voice tracker modulation
- Nya fält i `Voice`:
  - `tracker_pitch_cents: Cents`
  - `tracker_volume: NormalizedValue`
  - `tracker_panning: BipolarValue`
- Tracker pitch appliceras både på oscillatorfrekvens och SamplePlayer

#### SamplePlayer pitch_mod input
- Ny CV-input `pitch_mod` (semitones)
- Per-sample pitch modulation: `speed = base_speed * 2^(pitch_offset/12)`

#### ModuleGraph extern pitch modulation
- `set_sample_player_pitch_mod(semitones: f32)` - Sätter extern pitch modulation
- Automatisk injektion av pitch_mod buffer till SamplePlayer-moduler

#### Filer som ändrats
- `crates/synth_sequencer/src/events.rs` - `SequencerEvent::Modulation`
- `crates/synth_engine/src/sequencer_engine.rs` - Anropar `process_tick()`
- `crates/synth_engine/src/synth_engine.rs` - Hanterar `Modulation` events
- `crates/synth_engine/src/voice.rs` - Tracker modulation fält och applicering
- `crates/synth_engine/src/graph.rs` - Extern pitch mod infrastruktur
- `crates/synth_modules/src/sample_player.rs` - `pitch_mod` CV-input

---

## [0.58.0] - 2025-12-17
### Added - Tracker-Compatible Modules (MultiPointEnvelope & TrackerFilter)

Implementerade två nya moduler for forbattrad XM/IT tracker-kompatibilitet.

#### MultiPointEnvelope
Full XM/IT envelope-support med:
- Upp till 25 arbitrara punkter
- Linear interpolation mellan punkter
- Sustain-punkt (haller tills note-off)
- Loop-region (loopar medan sustained)
- Fadeout efter release
- Heap-fri implementation med ArrayVec

#### TrackerFilter
IT-kompatibelt resonant lowpass-filter med:
- Zxx-kontroll (0-127 cutoff/resonance)
- SVF (State Variable Filter) implementation
- Exponentiell cutoff-kurva (~110 Hz till ~10 kHz)
- Q-faktor 0.5-12.0

#### Nya typer i synth_core
- `EnvelopeFrame` - Envelope frame position (0-65535)
- `EnvelopeValue` - Envelope value (0.0-1.0)
- `EnvelopePointIndex` - Point index (0-24)
- `FadeoutRate` - Fadeout rate
- `TrackerCutoff` - Filter cutoff (0-127)
- `TrackerResonance` - Filter resonance (0-127)

#### XM Import Integration
- `ImportedInstrument` utokad med:
  - `envelope_points: Vec<(u16, f32)>` - Ra envelope-punkter
  - `envelope_sustain: Option<u8>` - Sustain-punkt index
  - `envelope_loop: Option<(u8, u8)>` - Loop region
  - `fadeout: f32` - Fadeout rate
- Debug import visar nu envelope-info per instrument

#### ModuleType registrering
- `ModuleType::MultiPointEnvelope`
- `ModuleType::TrackerFilter`

#### Filer som andrats
- `crates/synth_core/src/types/tracker.rs` - NY FIL: Alla tracker-newtypes
- `crates/synth_core/src/types/mod.rs` - Export tracker module
- `crates/synth_core/src/params/mod.rs` - Nya ModuleType varianter
- `crates/synth_modules/src/multi_point_envelope.rs` - NY FIL: MultiPointEnvelope
- `crates/synth_modules/src/tracker_filter.rs` - NY FIL: TrackerFilter
- `crates/synth_modules/src/lib.rs` - Export nya moduler
- `crates/modular_synth/src/io/import/mod.rs` - Utokad ImportedInstrument
- `crates/modular_synth/src/io/import/tracker.rs` - Extraherar envelope-data
- `crates/modular_synth/src/main.rs` - Debug output for envelope info
- `Cargo.toml` - arrayvec beroende

---

## [0.57.0] - 2025-12-17
### Added - CLI Debug Import Command

Ny kommandoradsparameter `--debug-import` / `-d` för att importera tracker-filer och visa debuginformation utan att starta GUI.

#### Användning
```bash
modular-synth --debug-import /path/to/song.xm
modular-synth -d /path/to/song.xm
```

#### Output
- Songinformation (namn, tempo, antal patterns/samples/instrument)
- Instrumentlista med sample-referenser och volym
- Samplelista med antal frames, kanaler och loop-info
- Arrangement med pattern-placeringar
- Första pattern med noter och duration-info
- Varningar för saknade samples eller ogiltiga instrument-referenser

#### Filer som ändrats
- `crates/modular_synth/src/main.rs` - Ny `CliAction` enum, `--debug-import` argument-parsing, `run_debug_import()` funktion

---

## [0.56.0] - 2025-12-17
### Fixed - XM Import Key Off and Effect-Only Rows

Fixade tre kritiska buggar i XM tracker import/uppspelning som gjorde att musiken lät fel.

#### Problemen
1. **Key Off ignorerades helt** - Noter utan explicit duration spelades för evigt (loopade stråkar, pads slutade aldrig)
2. **Effect-only rows ignorerades** - Rader med bara effekter (volume slides, vibrato continuation) kastades bort
3. **SetSpeed runtime ignorerades** - Tempo-ändringar via Fxx-effekt (x < 0x20) fungerade inte under uppspelning

#### Lösningen

**Key Off:**
- Lade till `last_note_index` och `last_note_start_tick` i `ChannelState` för att spåra aktiva noter
- Vid Key Off: beräknar duration från föregående not och sätter `note.duration`
- Vid ny not på samma kanal: implicit note-off på föregående not (tracker-beteende)

**Effect-only rows:**
- Ny `EffectOnlyEvent` typ i `synth_sequencer::pattern`
- Import skapar `EffectOnlyEvent` för rader utan noter men med effekter
- `SequencerEngine` emittar `SequencerEvent::Effect` för dessa

**SetSpeed runtime:**
- Nya fält `base_tempo` och `tracker_speed` i `SequencerEngine`
- `GlobalCommand::SetSpeed` justerar `cached_tempo` via formeln: `effective_tempo = base_tempo * (6 / speed)`
- Ny metod `recalculate_effective_tempo()` för konsistent tempo-hantering

#### Filer som ändrats
- `crates/synth_sequencer/src/pattern.rs` - Ny `EffectOnlyEvent` typ, `note_by_index_mut()`, `effect_events` fält
- `crates/synth_sequencer/src/lib.rs` - Export av `EffectOnlyEvent`
- `crates/synth_engine/src/sequencer_engine.rs` - SetSpeed runtime, effect-only event emission
- `crates/modular_synth/src/io/import/tracker.rs` - Key Off och effect-only row hantering

---

## [0.55.0] - 2025-12-12
### Fixed - XM/MOD Import Pitch Accuracy

Fixade pitch-beräkning för tracker-moduler genom att hantera FrequencyType (Amiga vs Linear).

#### Problemen
1. **FrequencyType ignorerades** - XM-filer kan använda Amiga eller Linear frekvensberäkning. Amiga C-4 = 8297 Hz vs Linear C-4 = 8363 Hz (~14 cents skillnad).
2. **Dubbel relative_pitch** - `relative_pitch` applicerades på både `sample_rate` OCH `root_note`, vilket tog ut varandra.
3. **Finetune ignorerades** - Sample finetune (-1..1 semitoner) användes inte alls.

#### Lösningen
- Beräknar korrekt basfrekvens baserat på `module.frequency_type` (Amiga: 8297 Hz, Linear: 8363 Hz)
- Applicerar `finetune` på sample_rate via formeln `2^(finetune/12)`
- Använder `relative_pitch` enbart för `root_note`
- Lade till `default_panning` fält i Sample struct

#### Filer som ändrats
- `crates/modular_synth/src/io/import/tracker.rs` - FrequencyType-hantering, finetune-fix
- `crates/synth_core/src/types/sample.rs` - Lade till `default_panning` fält

---

## [0.54.0] - 2025-12-12
### Fixed - Tracker Voice Allocation (TrackId Mismatch)

Fixade tracker-style voice allocation för korrekt mono-per-kanal uppspelning.

#### Problemet
Vid import av tracker-moduler (MOD/XM/S3M) skapades `TrackId` med `TrackId::new(channel_idx)` i `convert_pattern()`, men detta matchade inte de `TrackId`:s som skapats av `song.create_track()`. Resultatet var att `get_voice_index_for_track()` returnerade `None`, vilket orsakade polyfon allokering istället för tracker-style mono-per-kanal.

#### Lösningen
Skickade `track_ids` vektorn (från `song.create_track()`) till `convert_pattern()` och använde dessa ID:n istället för att skapa nya.

#### Filer som ändrats
- `crates/modular_synth/src/io/import/tracker.rs` - Skickar `track_ids` till `convert_pattern()`
- `crates/synth_engine/src/sequencer_engine.rs` - Förenklad `get_voice_index_for_track()`
- `crates/synth_engine/src/voice_allocator.rs` - Rensad debug-utskrift
- `crates/modular_synth/src/gui/egui_backend.rs` - Rensad debug-utskrift

---

## [0.53.0] - 2025-12-12
### Fixed - Rack View Silent Audio Bug

Kritisk fix för tyst ljud i Rack View efter graf-rebuild.

#### Problemet
Efter att ha valt ett instrument i Rack View producerade grafen inget ljud trots att oscillatorer och envelopes kördes korrekt. Endast den första audio-buffern efter note_on innehöll ljud.

#### Rotorsaken
`ModuleGraph::process_module()` återanvände `input_buffers` Vec mellan moduler men clearade bara **buffrarnas innehåll** - inte **listan av port-entries**.

Om modul A processades med port "in" och sedan modul B (t.ex. `StereoOutput`) också hade port "in", hittade den en **stale entry** från modul A istället för att skapa en ny från rätt koppling. Resultatet blev att signaler routades fel eller försvann.

#### Lösningen
Ändrade från att cleara buffrarnas innehåll till att cleara hela Vec:en med `self.input_buffers.clear()` vid början av varje modul-process.

#### Filer som ändrats
- `crates/synth_engine/src/graph.rs` - Fixade input_buffers hantering i `process_module()`

---

## [0.52.0] - 2025-12-12
### Changed - Cargo Workspace Refactoring

Delade upp monolitisk crate (~21k LOC) i 6 separata crates för bättre kompileringstider och arkitektur.

#### Ny Crate-struktur

```
modular-synth/
├── Cargo.toml (workspace root)
└── crates/
    ├── synth_core/      # Types, traits, audio abstractions
    ├── synth_dsp/       # DSP primitives (oscillators, filters)
    ├── synth_sequencer/ # Pattern, song, events
    ├── synth_modules/   # Synth modules and effects
    ├── synth_engine/    # Voice allocation, graph, sequencer engine
    └── modular_synth/   # GUI, main, audio backends
```

#### Beroendegraf

```
synth_core (types, traits)
    ↑
    ├── synth_dsp (oscillators, filters)
    ├── synth_sequencer (pattern, song)
    │
    └── synth_modules (oscillator, filter, effects)
            ↑
            └── synth_engine (graph, voice, instrument)
                    ↑
                    └── modular_synth (gui, main, cpal)
```

#### Fördelar

- **Snabbare inkrementell kompilering** - Ändringar i GUI behöver inte rekompilera DSP-kod
- **Bättre separation of concerns** - Tydliga gränser mellan moduler
- **Enklare testning** - Varje crate kan testas isolerat
- **Möjliggör framtida plugin-stöd** - `synth_engine` kan användas utan GUI

#### Borttagna filer

- `src/` mappen (all kod flyttad till `crates/`)
- `examples/` (flyttad till `crates/modular_synth/examples/`)

---

## [0.51.0] - 2025-12-12
### Added - Type Safety & Operator Overloading

Utökade typsystemet med nya domäntyper och operator overloading för renare DSP-kod.

#### Nya Cross-Type Operators

| Operation | Resultat | Användning |
|-----------|----------|------------|
| `Hertz / SampleRate` | `f32` | Phase increment för oscillatorer |
| `BipolarValue * NormalizedValue` | `f32` | LFO × mod depth (attenuering) |
| `SampleCount / SampleRate` | `Seconds` | Bufferstorlek → tid |

Exempel på förenklad kod:
```rust
// Före
let phase_inc = self.rate.as_f32() / self.sample_rate.as_f32();

// Efter
let phase_inc = self.rate / self.sample_rate;
```

#### Nya Typer (`src/types/audio.rs`)

- **`CpuUsage(f32)`** - CPU-belastning (0.0-1.0) med `is_warning()`, `is_critical()`
- **`StereoLevels`** - Peak-mätning för meters (`left`, `right`: Amplitude)
- **`TrackCount(usize)`** - Antal kanaler med konstanter: `MOD_STANDARD`, `THIRTYTWO`
- **`PatternIndex(usize)`** - Position i song arrangement
- **`RowIndex(u32)`** - Radnummer i pattern med `is_beat()`, `is_bar()`
- **`TrackerSpeed(u8)`** - Ticks per row (1-31, DEFAULT=6)

#### Uppdateringar till VoiceCount

- **`new()` clampar nu till 1-128** (var obegränsat)
- **`new_unchecked()`** för performance-kritiska paths
- **`THIRTYTWO`** konstant (32 voices för tracker)
- **`MAX_ALLOCATOR`** konstant (128 voices)
- **`clamp_allocator()`** metod för allocator range
- **`From<usize>`** implementation

#### Filer som ändrats
- `src/types/frequency.rs` - `Hertz / SampleRate` operator
- `src/types/normalized.rs` - `BipolarValue * NormalizedValue` operator
- `src/types/samples.rs` - `SampleCount / SampleRate` operator
- `src/types/audio.rs` - Nya typer och VoiceCount-uppdateringar

---

## [0.50.0] - 2025-12-12
### Added - Tracker Voice Allocation (Mono-Per-Channel)

Implementerade tracker-style voice allocation för korrekt MOD/XM/S3M-uppspelning.

#### Problemet
Importerade tracker-filer hade "ghost notes" - loopade ljud fortsatte spela även när nya noter startade på samma kanal. Tracker-moduler är mono-per-kanal, men synten använde polyfon röstallokering.

#### Lösningen
Ny `Tracker` allocation mode där varje kanal får en dedikerad röst (fixed voice index). Nya noter på samma kanal gör en legato-style retrigger utan att nollställa envelope.

#### Nya typer (`src/types/audio.rs`)
- **`VoiceIndex`** - Index för en specifik röst (0-255)
- **`VoiceCount`** - Antal röster med konstanter: `DUAL`, `QUAD`, `OCTO`, `SIXTEEN`, `THIRTYTWO`

#### TrackMode (`src/sequencer/track.rs`)
- **`TrackMode::MonoVoice(VoiceIndex)`** - Tracker-style, kanal i → röst i
- **`TrackMode::Polyphonic`** - Keyboard/MIDI-style, dynamisk allokering

#### Voice Allocator (`src/engine/voice_allocator.rs`)
- **`AllocationMode::Tracker`** - Ny allocation mode
- **`note_on_fixed_voice(voice_index, note, velocity)`** - Triggrar not på specifik röst
- **`note_off_fixed_voice(voice_index)`** - Släpper specifik röst
- **`resize(count)` / `resize_with_graph(count, template)`** - Dynamisk omstorlek av voice pool
- **`AllocatorConfig::max_voices`** är nu `VoiceCount` istället för `usize`

#### Sequencer Event
- **`NoteOn::voice_index: Option<VoiceIndex>`** - Specifierar vilken röst som ska användas

#### Import (`src/io/import/`)
- **`ImportedSong::min_voices`** - Minimum antal röster (antal tracker-kanaler)
- Tracker-import skapar nu en track per kanal med `TrackMode::MonoVoice`
- GUI skapar instrument med `AllocationMode::Tracker` och rätt antal röster

#### Filer som ändrats
- `src/types/audio.rs` - VoiceIndex, VoiceCount newtypes
- `src/engine/voice_allocator.rs` - Fixed voice allocation, resize
- `src/engine/sequencer_engine.rs` - voice_index routing
- `src/engine/synth_engine.rs` - note_on_fixed_voice() integration
- `src/sequencer/track.rs` - TrackMode enum
- `src/sequencer/events.rs` - voice_index i NoteOn
- `src/io/import/tracker.rs` - MonoVoice per kanal
- `src/io/import/mod.rs` - min_voices i ImportedSong
- `src/gui/egui_backend.rs` - Tracker mode vid import

---

## [0.49.0] - 2025-12-12
### Added - Debug System

Implementerade ett komplett debug-system för offline-analys av audio-motorn.

#### Nya komponenter

**SignalProbe** - Inspektera sample-värden vid specifika punkter
- Spela in samples från godtyckliga portar i grafen
- Beräkna statistik (RMS, peak, DC offset, min/max)
- Jämför signaler (korrelation, skillnad)
- Max-samples-gräns för att förhindra minnesläckor

**GraphDebugger** - Verifiera modulkopplingar
- Lista alla connections i grafen
- Sök path mellan moduler
- Hitta okopplade inputs/outputs
- Diagnostisera vanliga problem (envelope utan gate, amplifier utan CV)
- Human-readable graph description

**SequencerDebugger** - Stega genom songs tick-för-tick
- Stega framåt tick-för-tick offline
- Event-logg med tidsstämplar och källa
- Snapshot av aktiva noter och state
- Sammanfattning av genererade events

**VoiceDebugger** - Förstå voice allocation
- Spåra allokeringar, releases, steals
- State change-historik
- Snapshot av alla voices vid given tidpunkt
- Analysera voice reuse och unique voices

#### Feature flag

Debug-systemet är opt-in via `--features debug-tools`. Ingen overhead vid normal körning.

```toml
[features]
debug-tools = []
```

#### Filer som lagts till
- `src/debug/mod.rs` - Modul-struktur och re-exports
- `src/debug/signal_probe.rs` - SignalProbe implementation
- `src/debug/graph_debugger.rs` - GraphDebugger implementation
- `src/debug/sequencer_debugger.rs` - SequencerDebugger implementation
- `src/debug/voice_debugger.rs` - VoiceDebugger implementation

#### Dokumentation
- `docs/DEBUG_SYSTEM_DESIGN.md` - Design-dokument med API-specifikation

---

## [0.48.0] - 2025-12-12
### Fixed - Tracker Playback Volume (Silent XM Fix)

Kritisk fix för tyst uppspelning av importerade tracker-filer.

#### Problemet
Importerade XM/MOD-filer spelades nästan tyst (2-16% volym) trots korrekt import.

#### Rotorsaken
Volymen applicerades **dubbelt**:
1. Notens velocity (från volume-kolumnen) = t.ex. 0.12 (12%)
2. Effektprocessorns `SetVolume` = t.ex. 0.12 (12%)
3. Resultat: `0.12 × 0.12 = 0.014` (1.4% volym) - ohörbart!

#### Lösningen
I tracker-format representerar notens velocity volymkolumnens värde direkt.
Effektprocessorns volymmodulering ska endast påverka UNDER uppspelning (volume slide, tremolo), inte vid note-onset.

Ändrade `sequencer_engine.rs` att använda notens velocity direkt istället för att multiplicera med effektprocessorns volym.

#### Filer som ändrats
- `src/engine/sequencer_engine.rs` - Tog bort volym-multiplikation vid note-onset
- `src/engine/synth_engine.rs` - Rensade temporär debug-logging

---

## [0.47.0] - 2025-12-11
### Fixed - Multi-Channel Tracker Display & Sample Index

Fixar för korrekt visning av tracker-filer med många kanaler och sample-indexering.

#### Bugfixar
- **Multi-channel display:** Pattern visar nu alla kanaler (30 kanaler för joli_untouched.xm), inte bara 4
  - `Pattern.set_num_tracks()` - ny setter-metod för att sätta antal kanaler
  - Tracker-import sätter nu `num_tracks` baserat på modulens kanalantal
  - Tangentbordsnavigering i Sequencer-vyn respekterar nu antal kanaler från pattern
- **Sample-indexering:** Fixade bug där instrument utan samples fick felaktig sample_index
  - Kontrollerar nu `sample_count > 0` istället för `!instr.sample.is_empty()`
- **Standard volym:** `ChannelState.last_volume` sätts nu till 1.0 (full volym) som default

#### Filer som ändrats
- `src/sequencer/pattern.rs` - Lade till `set_num_tracks()` metod
- `src/io/import/tracker.rs` - Fixade sample-indexering och volym-default
- `src/gui/views/sequencer.rs` - Hämtar antal tracks från pattern

---

## [0.46.0] - 2025-12-11
### Added - Tracker Effects Implementation

Fullständig implementation av tracker-effekter för MOD/XM/S3M-import.

#### Effekter Implementerade
- **Volume Effects:** SetVolume, VolumeSlide, FineVolumeSlide, Tremolo
- **Pitch Effects:** Arpeggio, PortamentoUp, PortamentoDown, TonePortamento, Vibrato
- **Panning Effects:** SetPanning, PanningSlide
- **Timing Effects:** NoteDelay, NoteCut, NoteFadeOut, Retrigger
- **Global Effects:** SetTempo, SetSpeed, PatternBreak, PatternJump, PatternLoop, PatternDelay
- **Miscellaneous:** SampleOffset, FineTune, Glissando, VibratoWaveform, TremoloWaveform

#### Nya Typer (typsäkra wrappers)
- **`TrackerSpeed`** - Ticks per rad (default: 6)
- **`TickInRow`** - Position inom en rad
- **`TrackerSampleOffset`** - Startposition för sample-uppspelning
- **`PitchCents`** - Tonhöjdsförskjutning i cents
- **`PortamentoDirection`** - Up/Down/Off för portamento

#### Arkitektur
- **`ChannelEffectProcessor`** - Hanterar alla kanal-effekter
- **`ChannelEffectState`** - Per-kanal-tillstånd (volym, panning, vibrato-fas, etc.)
- **`ChannelModulation`** - Modulationsvärden som appliceras på noter
- **`GlobalCommand`** - Kommandon som påverkar sequencer-nivån

#### Effektkonvertering (xmrs → vår modell)
- `TrackEffect` → `EffectCommand` (not-nivå effekter)
- `GlobalEffect` → `EffectCommand` (pattern-nivå effekter)
- Waveform-mapping för vibrato/tremolo

#### Analysverktyg
- **`analyze_all_trackers`** - Analyserar alla tracker-filer i en katalog
  - Rapporterar vilka effekter som används
  - Identifierar saknade/ostödda funktioner
  - Listar envelope- och sample-features

```bash
cargo run --example analyze_all_trackers -- /path/to/music
```

#### Filer som ändrats
- `src/engine/tracker_effects.rs` - **NY** - Effektprocessing
- `src/engine/mod.rs` - Exporterar tracker_effects
- `src/engine/sequencer_engine.rs` - Integrerar effektprocessing
- `src/io/import/tracker.rs` - Konverterar xmrs-effekter till EffectCommand
- `examples/analyze_all_trackers.rs` - **NY** - Analysverktyg

---

## [0.45.0] - 2025-12-11
### Fixed - Tracker Import & Sequencer Playback

Kritiska fixar för tyst uppspelning av importerade tracker-moduler.

#### Bugfixar
- **Tyst XM-uppspelning:** `focused_instrument` rensas nu efter import så att alla instrument spelas i Sequencer-vyn
- **Fokuserad routing:** Sequencer-händelser respekterar nu `focused_instrument` för solo-läge i Rack-vyn

#### Nytt verktyg
- **`analyze_tracker`** - Diagnostikverktyg för MOD/XM/S3M-filer
  - Visar modulinfo (namn, BPM, tempo, kanaler, mönster)
  - Instrumentdetaljer (samples, volym-envelopes, pan, fadeout)
  - Sample-information (datalängd, volym, loop-inställningar)
  - Mönsterordning och noter i första mönstret
  - Sammanfattning med statistik

```bash
cargo run --example analyze_tracker -- /path/to/file.mod
cargo run --example analyze_tracker -- /path/to/file.xm
cargo run --example analyze_tracker -- /path/to/file.s3m
```

#### Filer som ändrats
- `src/gui/egui_backend.rs` - Rensar `focused_instrument` efter import till Sequencer-vy
- `src/engine/synth_engine.rs` - Kollapsade if-satser (clippy-fix)
- `examples/analyze_tracker.rs` - Nytt verktyg (ersätter analyze_xm.rs)

---

## [0.44.0] - 2025-12-11
### Added - Native File Dialogs

Ersätter manuella fildialog med `egui-file-dialog` crate för native filbläddrare.

#### File Dialog Integration
- **`egui-file-dialog` v0.12** - Native filväljare med filterstöd
- **Öppna patch** - Ny menypost "Open Patch..." för att öppna sparade patches
- **Spara patch** - "Save Patch..." öppnar filväljare med .json-filter
- **Import song** - "Import Song..." med filter för .mod/.xm/.s3m
- **Load Built-in** - Behållen för inbyggda example-patches

#### API-ändringar
- **`FileDialogMode`** enum: `OpenPatch`, `SavePatch`, `ImportSong`, `OpenSample`
- **`FileDialogResult`** enum: `Picked(PathBuf, mode)`, `Saved(PathBuf, mode)`
- **`DialogState`** metoder:
  - `open_open_patch_dialog()` - Öppna patch-fil
  - `open_save_patch_dialog(default_name)` - Spara patch
  - `open_import_song_dialog()` - Import tracker-fil
  - `open_sample_dialog()` - Öppna WAV-sample (förberett för framtida bruk)
  - `update_file_dialog(ctx)` - Hanterar dialog-state och returnerar resultat

#### Filer som ändrats
- `Cargo.toml` - `egui-file-dialog = "0.12"` tillagt
- `src/gui/dialogs.rs` - FileDialog integration, nya typer och metoder
- `src/gui/egui_backend.rs` - Uppdaterade menyer och FileDialogResult-hantering

---

## [0.43.0] - 2025-12-11
### Added - Focused Instrument & Tracker Playback Fixes

Lösningar för MIDI-kanalkonflikter och korrekt tracker-uppspelning.

#### Focused Instrument (Keyboard Routing)
- **Problem:** Med >16 instrument delade flera samma MIDI-kanal, alla spelade samtidigt
- **Lösning:** "Focused Instrument" - keyboard input (kanal 0) går endast till valt instrument
- **`EngineState`:** `focused_instrument: AtomicU32` för trådsäker state
- **`EngineCommand::SetFocusedInstrument`** - Nytt kommando för GUI-styrning
- **`EngineHandle`:** `set_focused_instrument()`, `get_focused_instrument()` metoder
- **GUI-integration:** Automatiskt fokus vid instrumentval, import, och borttagning

#### Mono-per-Track (Tracker-beteende)
- **Problem:** Loopade ljud försvann inte vid nya noter på samma track
- **Lösning:** `TrackId` newtype för typsäker track-identifiering
- **`Note::track`:** `Option<TrackId>` för mono-per-track routing
- **Sequencer:** `stop_notes_on_track()` stänger av föregående not automatiskt
- **Tracker-import:** Sätter `TrackId` för varje kanal vid import

#### Sample Player Fixes
- **Release Mode:** Non-looped samples använder nu `ReleaseMode::PlayToEnd` (default)
  - Drums/one-shots spelar klart istället för abrupt stopp
  - Looped samples behåller `ReleaseMode::Immediate`
- **Interpolation:** Konfigurerbar i GUI med Cubic som default
  - Nearest, Linear, Cubic, Hermite, Lagrange, Sinc8, Sinc16

#### Pattern View Improvements
- **Follow Playback:** Pattern-vyn följer nu aktiv rad under uppspelning
- **Auto-switch Pattern:** Automatiskt byte till aktivt pattern
- **Dynamiska tracks:** Visar rätt antal tracks från importerad fil (inte hårdkodat 4)
- **Pattern display:** Visar "1/3" format (aktuellt/totalt) istället för hex

#### Filer som ändrats
- `src/engine/state.rs` - `focused_instrument`, `NO_FOCUSED_INSTRUMENT`
- `src/engine/commands.rs` - `SetFocusedInstrument` kommando
- `src/engine/synth_engine.rs` - note routing, handle methods
- `src/sequencer/note.rs` - `track: Option<TrackId>`, `with_track()`
- `src/sequencer/ids.rs` - `TrackId` exporteras
- `src/engine/sequencer_engine.rs` - `ActiveNote::track`, `stop_notes_on_track()`
- `src/io/import/tracker.rs` - Sätter `TrackId` vid import
- `src/modules/sample_player.rs` - Smart `ReleaseMode` i `load_sample()`
- `src/gui/instrument_rack.rs` - Focused instrument vid val/borttagning
- `src/gui/egui_backend.rs` - Focused instrument vid start/import

---

## [0.42.0] - 2025-12-11
### Added - Song Playback & Sample Waveform Display

Uppspelning av importerade tracker-filer och visualisering av samples.

#### Transport-kontroll
- **`SetSong` kommando** - Skickar song till `SequencerEngine` för uppspelning
- **Transport-kommandon** i `synth_engine.rs`: `Play`, `Stop`, `Pause`, `Rewind`
- **GUI-knappar** i sequencer-vyn kopplade till engine

#### Sample Waveform-visning
- **`WaveformOverview`** utökad med stereo-stöd:
  - `peaks_left` / `peaks_right` för separata kanaler
  - `is_stereo` flagga
  - Nya metoder `peak_left_at()`, `peak_right_at()`
- **Ny widget** `draw_sample_waveform()` i `widgets/waveform_display.rs`:
  - Mono: Enkel centrerad vågform
  - Stereo: L/R separerade (ovanför/under mittlinje)
  - Stöd för playback-position indikator
- **Integration** i `patch_editor.rs` - Waveform visas automatiskt i SamplePlayer-moduler

#### Filer som ändrats
- `src/engine/commands.rs` - `SetSong` kommando
- `src/engine/synth_engine.rs` - Transport-hantering
- `src/engine/hub.rs`, `src/engine/transactions.rs` - SetSong stöd
- `src/gui/views/sequencer.rs` - `TransportAction` enum, knappkoppling
- `src/gui/egui_backend.rs` - Transport-routing, SetSong vid import
- `src/types/sample.rs` - Stereo `WaveformOverview`
- `src/gui/widgets/waveform_display.rs` - Ny widget
- `src/gui/patch_editor.rs` - Waveform i SamplePlayer
- `src/gui/module_panel.rs` - `waveform_overview` i `ModulePanelState`

---

## [0.41.0] - 2025-12-11
### Added - Tracker Import (MOD/XM/S3M)

Import av klassiska tracker-filer direkt i synthen.

#### Import-arkitektur (`src/io/import/`)

- **`SongImporter` trait** - Utökningsbar arkitektur för filformat:
  - `name()` - Importernamn
  - `extensions()` - Stödda filändelser
  - `can_import()` - Formatdetektering
  - `import()` - Returnerar `ImportResult<ImportedSong>`

- **`ImportedSong`** - Resultat med:
  - `song: Song` - Konverterade patterns och arrangement
  - `samples: Vec<Arc<Sample>>` - Extraherade samples

- **`ImportError`** enum - Typade fel (NotFound, Io, UnsupportedFormat, Parse, InvalidData)

#### Tracker-loader (`src/io/import/tracker.rs`)

- **`TrackerImporter`** - Stöd för MOD, XM, S3M via `xmrs` crate
- **Sample-konvertering**:
  - 8-bit: `i8 / 128.0` → `SampleValue`
  - 16-bit: `i16 / 32768.0` → `SampleValue`
  - Stereo och float-format stöds
- **Pattern-konvertering**:
  - xmrs `TrackUnit` → interna `Note`
  - Beräknar ticks per rad baserat på tracker-speed
  - Hanterar `Pitch::None` och `Pitch::Off` (keyoff)
  - Sätter `RowResolution` för korrekt tracker-visning
- **Tempo-hantering**: Använder modulens `default_bpm` och `default_tempo`

#### GUI-integration

- **File → Import Song...** menyval
- **Import Song-dialog** (`src/gui/dialogs.rs`):
  - Sökvägs-inmatning
  - Validering (filen måste finnas)
  - Stödda format visas (.mod, .xm, .s3m)
- **Automatisk vy-växling** till Sequencer efter import
- **Status-toast** visar filnamn och antal samples

#### Beroenden

- `xmrs = { version = "0.9", features = ["std", "import"] }`

---

## [0.40.0] - 2025-12-10
### Added - The Hybrid Tracker

En modern tracker-arkitektur med View Adapter-mönster för framtida Piano Roll-stöd.

#### Datastruktur (`src/sequencer/pattern.rs`)

- **`TrackCell` enum** - Cell-baserad representation för tracker-style editing:
  - `Empty` - Tom cell
  - `Note { pitch, instrument, velocity }` - Not-event
  - `NoteOff` - Not-avstängning (`===`)
  - `Effect { command, value }` - Effekt-cell

- **`TrackerGrid`** - Grid-baserad lagring (rows × tracks):
  - `get()` / `set()` / `clear()` - Cell-operationer
  - `resize()` - Dynamisk storlek
  - `effects()` - Effekt-kolumner per cell

- **Pattern dual representation**:
  - `notes: Vec<Note>` - Piano roll-format (start, duration)
  - `grid: Option<TrackerGrid>` - Tracker-format (lazy-initialized)
  - `sync_grid_from_notes()` / `sync_notes_from_grid()` - Synkronisering

#### View Adapter (`src/sequencer/view/render.rs`)

- **`ColumnType`** enum: `RowIdx`, `Note`, `Instrument`, `Volume`, `EffectType`, `EffectValue`
- **`render_cell_text()`** - Returnerar `Cow<'static, str>` för noll-allokering på statiska strängar
- **`cell_color()`** - Färgbestämning baserat på cell-typ och cursor
- **`draw_track_cell()`** - Ny rendering via View Adapter
- **`draw_tracker_grid_from_pattern()`** - Optimerad rendering direkt från TrackerGrid

#### Input & Fokus (`src/sequencer/view/input.rs`)

- **`TrackerCommand`** enum - Engine-kommunikation (SetNote, SetNoteOff, ClearCell, etc.)
- **`TrackerCursor`** - Cursor-position (row, track, column)
- **`key_to_semitone()`** - Piano-keyboard layout (Z=C, S=C#, Q=C+12)
- **`hex_char_to_value()`** - Hex-inmatning för instrument/volym

#### Prestanda

- Virtual scrolling via `egui_extras::TableBuilder`
- `Cow<str>` för statiska strängar (---,  ===, ..) undviker heap-allokering
- Beat-markering var 4:e rad

#### Nya tester

- `test_track_cell_creation`
- `test_tracker_grid_basic`
- `test_tracker_grid_resize`
- `test_pattern_grid_sync`
- `test_pattern_set_cell`
- `test_grid_note_off_handling`
- Input-tester: `test_key_to_semitone`, `test_to_midi_note`, `test_hex_char_to_value`

---

## [0.39.0] - 2025-12-10
### Changed - Eliminera Duplicerade Typnamn

Alla strukturer och typer med samma namn i olika moduler har bytt namn till unika, beskrivande namn.

- **DSP-typer:**
  - `dsp::FilterType` → `SvfFilterType` (SVF = State Variable Filter)

- **Sequencer-typer:**
  - `sequencer::InstrumentId` → `SeqInstrumentId` (+ type alias för bakåtkompatibilitet)
  - `sequencer::InstrumentParam` → `AutoInstrumentParam` (automation-relaterad)

- **Engine shared state:**
  - `shared_state::MeterState` → `SharedMeterState` (trådsäker version)
  - `shared_state::TransportState` → `SharedTransportState` (trådsäker version)

- **Patch-serialisering:**
  - `patch::ModuleType` → `PatchModuleType`
  - `params::to_module_type()` → `to_patch_module_type()`

- **GUI widgets:**
  - `widgets::Port` → `PortWidget`
  - `widgets::PortType` → `WidgetPortType`
  - `widgets::PortDirection` → `WidgetPortDirection`
  - `patch_editor::VisualizerType` → `PaletteVisualizerType`

- **SampleRate:**
  - Behålls som två typer: `audio::SampleRate(u32)` för hårdvara, `types::SampleRate(f32)` för DSP
  - `From`-implementationer finns för konvertering mellan dem

### Why
Eliminerar förvirring vid import och gör det tydligt vilken typ som avses i varje kontext.

---

## [0.38.0] - 2025-12-10
### Added - Typsäkra Newtypes i Core Traits

Fullständig typning av `ProcessContext`, `PolyModule` och `AudioEffect` med newtype-mönstret.

- **Nya typer (`src/types/`):**
  - `BeatPosition` - Position i beats (musikalisk tid, f64)
    - Metoder: `bar()`, `beat_in_bar()`, `to_seconds()`, `quantize()`, `advance_samples()`
    - Konstant: `ZERO`
  - `Velocity` - MIDI velocity (0.0-1.0)
    - Metoder: `from_midi()`, `to_midi()`, `curve()`, `scale()`, `lerp()`
    - Konstanter: `ZERO`, `MAX`, `DEFAULT`, `PIANO`, `FORTE`
  - `MidiChannel` - MIDI-kanal (1-16)
    - Metoder: `as_u8()`, `as_index()`
    - Konstanter: `CH1`, `DRUMS`

- **Uppdaterad `ProcessContext`:**
  - `samples: usize` → `samples: SampleCount`
  - `position_beats: f64` → `position_beats: BeatPosition`

- **Uppdaterad `PolyModule` trait:**
  - `note_on(_note: MidiNote, _velocity: f32)` → `note_on(_note: MidiNote, _velocity: Velocity)`
  - `set_sample_rate(_sample_rate: f32)` → `set_sample_rate(_sample_rate: SampleRate)`

- **Uppdaterad `AudioEffect` trait:**
  - `set_mix(mix: f32)` → `set_mix(mix: NormalizedValue)`
  - `get_mix() -> f32` → `get_mix() -> NormalizedValue`
  - `tail_samples() -> usize` → `tail_samples() -> SampleCount`
  - `set_sample_rate(_sample_rate: f32)` → `set_sample_rate(_sample_rate: SampleRate)`

### Changed
- Alla 8 effekter uppdaterade med typsäkra signaturer
- Alla moduler som implementerar `PolyModule` uppdaterade
- `VoiceState`, `VoiceAllocator`, `Voice` använder nu `Velocity`
- Engine-filer använder typsäkra typer genomgående

---

## [0.37.0] - 2025-12-10
### Added - ValueRange Typ för Parameterhantering

- **Ny `ValueRange`-typ (`src/types/range.rs`):**
  - Kapslar in `min`, `max`, `default` i en enda typ
  - Fördefinierade konstanter: `UNIT`, `UNIT_ZERO`, `UNIT_ONE`, `BIPOLAR`, `PERCENT`, `TOGGLE`
  - Konstruktorer: `new()`, `symmetric()`, `from_min()`, `from_max()`
  - Metoder: `span()`, `contains()`, `clamp()`, `normalize()`, `denormalize()`, `lerp()`

- **Uppdaterad `ParameterDescriptor`:**
  - Ersatte separata `min`, `max`, `default` fält med `range: ValueRange`
  - Ny `value_range()` builder-metod
  - Bakåtkompatibla accessor-metoder: `min()`, `max()`, `default_value()`

- **Uppdaterad `ResponseCurve`:**
  - `normalize()` och `denormalize()` tar nu `ValueRange` istället för separata parametrar

### Changed
- GUI widgets (`Knob`, `module_panel`, `patch_editor`) använder nu `param.range.default/min/max`
- `ParameterWidget` i `src/ui/mod.rs` använder `range.normalize()` och `range.span()`

---

## [0.36.1] - 2025-12-10
### Changed - Kompaktare Modulstorlekar

- **Minskade modulstorlekar för att matcha kompakta widgets:**
  - Min bredd: 180px → 140px
  - Min höjd: 100px → 80px
  - Auto-layout gap: 10px → 8px
  - Modul X-offset: 210px → 160px
  - Modul Y-offset: 320px → 200px

- **Uppdaterade visualizer-storlekar:**
  - Meter: 80x100 → 60x80
  - Oscilloscope: 160x80 → 120x60
  - ADSR: 140x50 → 120x50

---

## [0.36.0] - 2025-12-10
### Added - ADSR Envelope Editor & Kompakta Knobs

- **Interaktiv EnvelopeEditor (`src/gui/widgets/envelope.rs`):**
  - Draggbara kontrollpunkter för Attack, Decay, Sustain, Release
  - Grid-bakgrund (5x5 linjer) för visuell referens
  - Tooltips visar värden vid hover/drag (ms/s för tid, % för sustain)
  - Total ljudtid (Σ A+D+R) visas i övre högra hörnet
  - Dynamisk skalning anpassar sig efter faktiska värden
  - Glow-effekt runt aktiva kontrollpunkter

- **Kompakta Knobs (`src/gui/widgets/knob.rs`):**
  - Storlek minskad: 72px → 36px (default), 56px → 28px (small), 88px → 48px (large)
  - Värde visas nu som tooltip istället för text i mitten
  - Borttagen yttre ram för kompaktare utseende
  - Arc-bredd och indikator skalas med knob-storlek

- **Återanvändbar Tooltip-modul (`src/gui/widgets/tooltip.rs`):**
  - `draw_value_tooltip()` - generell tooltip på valfri position
  - `draw_tooltip_right_of()` - för knobs (höger om cirkeln)
  - `draw_tooltip_above()` - för envelope-punkter
  - Ritas på `Order::Tooltip`-lagret för att alltid visas överst

- **Förbättrad Port-layout (`src/gui/patch_editor.rs`):**
  - Inputs vänsterställda, outputs högerställda med flexibelt mellanrum
  - Mindre labels (9px) för kompaktare vy
  - Tooltips med fullständig beskrivning vid hover

- **Förbättrade Topbar-knappar:**
  - Power: `●`/`○` med grön/grå färg och detaljerad hover-text
  - Connectivity: `◆`/`◇` med färgkodning och förklaringar
  - Större klickyta (20x20px) för bättre användbarhet

### Changed
- ADSR-moduler använder nu EnvelopeEditor istället för sliders
- Endast knob-parametrar (Vel Sens, kurvor) visas under envelope-editorn

---

## [0.35.0] - 2025-12-10
### Added - Tracker View (FastTracker II-inspirerad sequencer)
- **Ny TrackerViewState (`src/sequencer/view/state.rs`):**
  - Stark typ `RowIndex` för typsäker rad-navigering
  - `TrackerColumn` enum: Note, Instrument, Volume, EffectType, EffectValue
  - `TrackerViewState` med: cursor_row, cursor_track, cursor_column, octave, step_size, etc.
  - Hjälpmetoder: `cursor_down/up/left/right`, `ensure_cursor_visible`, `octave_up/down`

- **Tracker Grid Rendering (`src/sequencer/view/render.rs`):**
  - Använder `egui_extras::TableBuilder` för virtuell scrollning
  - `draw_tracker_grid()` - renderar pattern med färgkodade celler
  - `TrackerColors` - anpassningsbara färger för tracker-vyn
  - Stöd för Note, Instrument, Volume och Effect-kolumner

- **Sequencer View (`src/gui/views/sequencer.rs`):**
  - Toolbar med: Transport (⏮▶⏹⏺), Octave-väljare (F1/F2), Step-väljare, Follow-toggle
  - Full tangentbordsnavigering: Piltangenter, Home/End, PageUp/Down, Tab
  - Piano-tangentbordslayout för noter: Z=C, S=C#, X=D... Q=C+1, W=D+1, etc.
  - Placeholder för "No Song" med kortkommandon

- **Beroenden:**
  - Lagt till `egui_extras = "0.33"` för TableBuilder

---

## [0.34.11] - 2025-12-09
### Fixed - Auto Layout: ADSR/Modulation-moduler håller sig inom vyn
- **Korrigerad beräkning av modulation-radens Y-position:**
  - `mod_row_index` sätts nu korrekt till `main_rows` (efter alla huvudmoduler)
  - `mod_base_y` begränsas med `.min(max_y)` för att garantera att modulen håller sig inom canvas
- **ADSR/Envelope-moduler överlappar inte längre pianot/keyboardet**

---

## [0.34.10] - 2025-12-09
### Fixed - Auto Layout: Moduler håller sig garanterat inom vyn
- **Modulhöjden anpassas nu automatiskt:**
  - Om alla rader inte får plats med MIN_MODULE_HEIGHT (140px), krymps modulerna
  - Absolut minimum 60px höjd för att alltid passa
  - Formeln: `((available_height - GAP) / total_rows - GAP).max(60.0)`
- **Undersidan av moduler går aldrig utanför canvas:**
  - `max_y` beräknas som `available_rect.max.y - module_height - GAP`
  - `clamp_pos()` garanterar att modulens position + höjd alltid är inom vyn
- **Inga moduler överlappar keyboard/piano**

---

## [0.34.9] - 2025-12-09
### Improved - Auto Layout med strikta gränser
- **Okopplade moduler hanteras separat:**
  - Moduler utan kopplingar placeras nu i högra kolumnen
  - Staplas vertikalt för att undvika överlappning
- **Strikt gränskontroll:**
  - `clamp_pos()` funktion säkerställer att ALLA moduler håller sig inom canvas
  - Inga moduler kan överlappa pianot/keyboardet
  - `max_x` och `max_y` beräknas från modulstorlek
- **Förbättrad modulkategorisering:**
  - Kopplade huvudmoduler: vänster-till-höger efter signalflöde
  - Kopplade modulationsmoduler: under deras mål
  - Okopplade moduler: egen kolumn till höger
- **Nya tester:**
  - `test_disconnected_modules_in_corner` - verifierar att okopplade moduler placeras rätt
  - `test_linear_chain_within_bounds` - verifierar gränser

---

## [0.34.8] - 2025-12-09
### Improved - Auto Layout fyller hela vyn
- **Algoritmen omskriven för att dynamiskt beräkna modulstorlek:**
  - Modulbredden beräknas så att alla kolumner fyller tillgänglig bredd
  - Modulhöjden beräknas så att alla rader (inklusive modulation) fyller tillgänglig höjd
  - Minimumstorleks-begränsningar: 150x120 px
- **Moduler håller sig nu inom vyn:**
  - Canvas-rektangeln (exklusive sidopaneler och keyboard) används som begränsning
  - Modulation placeras under huvudsignalvägen med 25px gap
- **Förenklad API:**
  - `calculate_layout()` tar nu `Rect` direkt istället för `LayoutConfig`
  - Alla beräkningar sker internt baserat på tillgänglig yta

---

## [0.34.7] - 2025-12-09
### Improved - Auto Layout toolbar och höjdberäkning
- **Auto Layout-knappen ritas nu i foreground layer** - syns alltid överst, även när moduler dras över
- **Förbättrad layoutalgoritm:**
  - Moduler på samma djup-nivå staplas nu vertikalt (samma kolumn)
  - Djup-nivåer fortsätter till nästa kolumn
  - Om kolumnerna tar slut, fortsätter layouten på en ny "rad av kolumner"
- **Korrekt höjdhantering:**
  - Modulationsmoduler placeras baserat på faktiskt antal rader som används
  - Begränsning till `max_main_rows + 1` för att undvika att modulation hamnar för långt ner

---

## [0.34.6] - 2025-12-09
### Improved - Auto Layout respekterar tillgängligt utrymme
- **Algoritmen tar nu hänsyn till:**
  - Den tillgängliga canvas-ytan (exklusive sidopaneler)
  - Modulernas storlek (200x180 px)
  - Gap mellan moduler (20 px)
  - Radbrytning när kolumner inte får plats
- **Nya konfigurations-parametrar:**
  - `area_min`, `area_max` - Tillgänglig layoutyta
  - `module_size` - Modulstorlek för beräkningar
  - `gap_x`, `gap_y` - Avstånd mellan moduler
  - `modulation_gap` - Extra avstånd till modulations-raden
- **Moduler överlappar inte längre** - algoritmen placerar moduler i ett rutnät
- **Canvas-rect skickas nu till layoutfunktionen** för korrekt placering

---

## [0.34.5] - 2025-12-09
### Fixed - Auto Layout-knappen fungerar nu
- **Problemet:** Auto Layout-knappen uppdaterade interna positioner men egui:s `Window`-widget ignorerade detta eftersom `default_pos()` bara sätter positionen första gången fönstret ritas.
- **Lösningen:** Lade till `needs_reposition: HashSet<ModuleId>` i `PatchEditor` som markerar moduler som behöver omplaceras. När en modul är markerad används `current_pos()` istället för `default_pos()`, vilket tvingar fönstret till den nya positionen.
- Auto Layout fungerar nu som förväntat - moduler flyttas till sina beräknade positioner baserat på signalflödet.

---

## [0.34.4] - 2025-12-09
### Added - Workspace GUI Navigation
- **gui/app/state.rs** - Nya navigations-enums:
  - `AppView` - Rack/Sequencer/Mixer vy-val med `icon()` och `label()` metoder
  - `TopPanel` - None/Midi/Engine för expanderbara paneler (förberett för framtida funktionalitet)
- **gui/views/layout.rs** - Top bar och drawer komponenter:
  - `TopBarContext` struct för att skicka state till top bar
  - `draw_top_bar()` - Komplett menybar med vy-flikar
  - `draw_top_drawer()` - Expanderbar panel för MIDI/Engine (förberedd)
- **gui/views/rack.rs** - Rack-vy komponent:
  - `RackContext` struct för instrument rack state
  - `draw_instrument_rack()` och `draw_empty_state()` funktioner
- **gui/views/sequencer.rs** - Sequencer-vy (placeholder)
- **gui/views/mixer.rs** - Mixer-vy (placeholder)

### Changed - View Routing
- **egui_backend.rs** - Vy-router implementerad:
  - `active_view: AppView` fält i `SynthApp`
  - Vy-flikar i menyraden (🎛️ Rack, 🎹 Sequencer, 🎚️ Mixer)
  - Toolbar och instrument rack visas endast i Rack-vyn
  - Match-statement för vy-routing i CentralPanel

---

## [0.34.3] - 2025-12-09
### Improved - Prestandaoptimering & Slutfört Type Hardening

**Del 1: InternPool optimering (prestandakritisk)**
- `PortName` har nu **compile-time konstanter** (`PortName::IN`, `PortName::OUT`, etc.)
- **Ingen låsning krävs** för standardportnamn i ljudtråden
- 23 fördefinierade port-IDn: `IN`, `OUT`, `IN_L`, `IN_R`, `OUT_L`, `OUT_R`, `FREQ`, `FREQ_CV`, `GATE`, `CUTOFF_CV`, `RESONANCE_CV`, `PWM`, `FM`, `PM`, `SYNC`, `LEVEL`, `PAN`, `RATE_CV`, `CV`, `PAN_CV`, `LEFT`, `RIGHT`, `VELOCITY`
- Gamla metoder (`input()`, `output()`, etc.) är deprecated

**Del 2: InputPorts::get optimering**
- `InputPorts::get(name: PortName)` - direkt `u32`-jämförelse, **O(1) utan strängjämförelse**
- `InputPorts::get_str(name: &str)` - convenience-metod för dynamiska portnamn
- Alla moduler uppdaterade att använda `PortName::*` konstanter

**Del 3: Nya state enums**
- `FreezeState` - `Unfrozen`/`Frozen` för reverb/oscilloscope freeze
- `Polarity` - `Normal`/`Inverted` med `multiplier()` metod
- `TempoSyncState` - alias för `SyncMode`

**Del 4: Modulmigrering bool → enum**
- `KeyboardPanner`: `invert: bool` → `polarity: Polarity`
- `Delay`: `tempo_sync: bool` → `tempo_sync: TempoSyncState`
- `Mixer`: `mute: bool` → `mute_state: MuteState`, `limit: bool` → `limit_mode: LimitMode`

**Del 5: Parametrar uppdaterade**
- `KeyboardPannerParam::Invert(Polarity)` istället för `bool`
- Alla `as_f32()` och `with_f32()` metoder uppdaterade

**Del 6: SamplePlayer (redan optimerad)**
- `Arc::clone` sker redan innan for-loopen - korrekt implementerad

---

## [0.34.2] - 2025-12-09
### Improved - Fas 3 & 4: Type Hardening Complete
- **Fas 3 - Arkitektur & Prestanda:** Verifierad - redan optimerad
  - `PortName` (internad sträng) redan implementerad i `types/interned.rs`
  - `EffectCommand` enum redan typat i `sequencer/effects.rs`
  - HashMap-lookups använder `&str` utan allokeringar i audio-tråden
- **Fas 4 - FilterState wrapper:**
  - `Filter` (SVF) använder nu `FilterState` istället för råa `f32`
  - `ic1eq` och `ic2eq` integrator-state är nu typsäkra
  - Konsistent med `LadderFilter` som redan använde `FilterState`

---

## [0.34.1] - 2025-12-09
### Improved - Fas 2: Sampling & Uppspelning
- **types/sample.rs** - `Interpolation` enum utökad med GUI-stöd:
  - `ALL` konstant med alla 7 interpolationslägen
  - `name()`, `id()`, `index()`, `from_index()` metoder
  - `to_choices()` för GUI-dropdown
- **types/state.rs** - `NoteReleaseState` enum tillagd:
  - `Held` - Ton hålls, normal uppspelning med looping
  - `Released` - Ton släppt, spelar till slut utan looping
  - Metoder: `is_released()`, `is_held()`, `release()`, `hold()`
- **SamplePlayer** - Refaktorerad med semantisk state:
  - `note_release_state: NoteReleaseState` istället för `releasing: bool`
  - Tydligare kodintention vid note-on/note-off hantering

---

## [0.34.0] - 2025-12-09
### Added - Type Hardening: Semantic State Enums
- **types/state.rs** - Ny modul för semantiska tillstånds-enums som eliminerar "Boolean Blindness"
  - `EnableState` - Enabled/Disabled
  - `MuteState` - Unmuted/Muted
  - `SoloState` - Normal/Solo
  - `BypassState` - Active/Bypassed
  - `SyncMode` - Free/TempoSync
  - `RetriggerMode` - Continue/Retrigger
  - `ClipMode` - Off/Soft/Hard
  - `LimitMode` - Enabled/Disabled
- **types/sample.rs** - Flyttade `LoopMode` och `ReleaseMode` från engine/params/modules.rs
  - Tillhör nu types-modulen där sampling-relaterade typer samlas
  - Bakåtkompatibla re-exports i engine/params/modules.rs

### Changed - Module Refactoring with State Enums
- **SharedEngineState** - `ModuleStateSnapshot` använder nu:
  - `bypass_state: BypassState` istället för `bypassed: bool`
  - `mute_state: MuteState` istället för `muted: bool`
  - `solo_state: SoloState` istället för `solo: bool`
- **Amplifier** - `clip_mode: ClipMode` istället för `soft_clip: bool`
  - Stöd för Off, Soft (tanh), och Hard clipping
- **LFO** - Använder nu:
  - `sync_mode: SyncMode` istället för `tempo_sync: bool`
  - `retrigger_mode: RetriggerMode` istället för `retrigger: bool`
- **StereoOutput** - Använder nu:
  - `mute_state: MuteState` istället för `muted: bool`
  - `limit_mode: LimitMode` istället för `limit_enabled: bool`

---

## [0.33.27] - 2025-12-09
### Improved - GUI Views Module
- **gui/views/** - Ny modul för återanvändbara GUI-komponenter
  - `views/master_effects.rs` - `MasterEffectParams` och `MasterEffectUiState` typer
  - `views/meters.rs` - `draw_meter()` och `draw_meter_horizontal()` funktioner
  - Reducerade egui_backend.rs från 2698 till 2482 rader (~216 rader flyttade)

---

## [0.33.26] - 2025-12-09
### Improved - Code Organization Refactoring
- **gui/input.rs** - Ny modul för keyboard input hantering
  - Extraherade `KEY_MAP` konstant och `handle_keyboard_input()` från egui_backend.rs
  - `KeyboardInputState` struct för att samla input state innan mutation
  - Reducerade egui_backend.rs med ~65 rader
- **SynthEngine command handlers** - Refaktorerade 620-raders `handle_command()` match-block
  - Extraherade 30+ handler-metoder grupperade i kategorier:
    - Instrument management (add, remove, set params, channel, enabled, solo)
    - Note control (note on, note off, all notes off)
    - MIDI controllers (pitch bend, mod wheel, aftertouch, poly aftertouch)
    - Global parameters (master volume, glide time)
    - Voice/module parameters (set voice param, set module param)
    - Reset/clear (reset, clear all modules)
    - Effects (bypass, effect param, enabled, visualizers, add/remove effect)
    - Modular routing (add/remove module, connect, disconnect)
  - Tydligare kodseparation och enklare underhåll

---

## [0.33.25] - 2025-12-09
### Added - StereoSample Type & DSP Module
- **StereoSample** - Ny typ i `types/audio.rs` för stereo-samples
  - Ersätter `(f32, f32)` och `[f32; 2]` för stereosignaler
  - Metoder: `new()`, `from_mono()`, `apply_gain()`, `apply_pan()`, `to_mono()`, `mix()`, `soft_clip()`, `hard_clip()`, `peak()`
  - Implementerar `Add`, `Sub`, `Mul<f32>`, `From<(f32, f32)>`, `From<[f32; 2]>`
- **src/dsp/** - Ny modul för återanvändbara DSP-primitiver
  - `dsp/oscillators.rs` - `poly_blep()` och `poly_blep_integrated()` för band-limiterade vågformer
  - `dsp/filters.rs` - `SvfCoeffs` och `BiquadCoeffs` för filterberäkningar
  - `dsp/delay.rs` - `DelayLine` och `InterpolatedDelayLine` för delay-effekter

### Improved - Module Refactoring
- **KeyboardPanner** - Använder nu `StereoSample::apply_pan()` internt
- **StereoOutput** - Refaktorerad att använda `StereoSample` för beräkningar
- **Delay effect** - Använder nu `StereoSample` för stereobearbetning
- **Reverb effect** - Använder nu `StereoSample` för stereobearbetning
- **Oscillator** - Använder nu `dsp::oscillators::poly_blep()` istället för lokal metod

---

## [0.33.24] - 2025-12-09
### Improved - Type Methods & Sequencer Types
- **MidiNote::transpose** - Flyttade transponeringslogik till typen, returnerar `Option<MidiNote>`
- **Pitch::transpose** - Sequencer-typ uppdaterad till `Semitones`, returnerar `Option<Pitch>`
- **PatternPlacement** - `transpose` nu `Semitones`, `gain` nu `Gain`
- **TempoChange/Song** - `bpm` och `default_tempo` nu `Bpm` istället för `f32`
- **Pattern::generate_events** - Använder nu `Semitones` för transponering
- **Instrument::transpose_note** - Delegerar nu till `MidiNote::transpose`

---

## [0.33.23] - 2025-12-09
### Improved - Strict Type Hardening
- **StereoBalance** - Ny typ för stereopanorering med constant-power gains
- **KeyboardPanner** - Använder nu `MidiNote`, `BipolarValue`, `StereoBalance` istället för primitiver
- **BodyResonance** - Filterstate arrays använder nu `FilterState` istället för `f32`
- **Instrument transpose** - Använder nu `Semitones` istället för `i8`
- **InstrumentParam::Transpose** - Uppdaterad till `Semitones`
- **Oscilloscope** - `mix` och `sample_rate` använder nu `NormalizedValue` och `SampleRate`
- **LevelMeter** - `mix` och `sample_rate` använder nu `NormalizedValue` och `SampleRate`
- **GUI InstrumentUiState** - `transpose` använder nu `Semitones`

---

## [0.33.22] - 2025-12-09
### Optimized - GUI Rendering with egui Shape Primitives
- **Kablar** - Ersatte manuell Bézier-loop (32 segment × 3 lager) med `CubicBezierShape`
- **Oscilloskop** - Ersatte ~200 `line_segment()` med en `Shape::line()`
- **Waveform-väljare** - Ersatte 31 `line_segment()` per ikon med `Shape::line()`
- **ADSR Envelope** - Ersatte 4 `line_segment()` med `Shape::line()`
- **Draw Call Batching** - GPU kan nu rita alla punkter i en operation
- **Automatisk LOD** - `CubicBezierShape` hanterar detaljnivå automatiskt

---

## [0.33.21] - 2025-12-09
### Improved - Enhanced Sample Player
- **PlaybackState** - Ersatte `bool` med typat enum för tydligare state
- **SampleName** - Newtype för sample-namn istället för rå `String`
- **ReleaseMode** - Tre lägen: `Immediate`, `PlayToEnd`, `PlayToLoop` för flexibel note-off hantering
- **Velocity Sensitivity** - Parameter för hur mycket velocity påverkar volym (0-100%)
- **Nya interpolationer** - Hermite, Lagrange, Sinc8, Sinc16 för högkvalitativ uppspelning
- **Loop Crossfade** - 0-50ms crossfade vid loop-punkter för klickfri looping
- **Root Key Detection** - Automatisk detektion av grundton från filnamn (t.ex. "Piano_C3.wav")
- **WaveformOverview** - Pre-beräknad waveform för effektiv visualisering
- **PlaybackPositionBuffer** - Atomic position buffer för lock-free GUI-synk

### Changed
- Konsoliderade `LoopMode` - endast en definition i params/modules.rs
- Tog bort `loop_mode` från `Sample` struct (hör till spelaren, inte samplen)

---

## [0.33.20] - 2025-12-09
### Added - Sample Player & Sample Manager
- **SamplePlayer** - Ny modul för uppspelning av WAV-samples
  - Pitch tracking (transponerar automatiskt baserat på spelade noter)
  - Loop modes: Off, Forward, Backward, PingPong
  - Start/End positions för sample trimming
  - Loop Start/End för preciserad loop-region
  - Speed-kontroll (0.1x - 4.0x)
  - Interpolation: Nearest, Linear, Cubic (Catmull-Rom)
  - Stereo och mono-samples stöds
- **SampleManager** - GUI-thread sample loader med caching
  - Laddar WAV-filer (8/16/24/32-bit int, 32-bit float)
  - Cache förhindrar dubbel-laddning av samma fil
  - Thread-safe via `Arc<Sample>`
- **Nya typer** - `SampleValue`, `PlaybackPosition`, `SampleIndex`, `PlaybackSpeed`, `ChannelMode`, `Interpolation`, `PlaybackDirection`
- **hound** - Nytt beroende för WAV-läsning

---

## [0.33.19] - 2025-12-08
### Added - Improved Cables & Auto-Layout
- **Kablar med gravitation** - Kablar hänger nedåt med naturlig "sag" (15% av avståndet)
- **Skuggor** - Svart skugga under kablar för djupkänsla
- **Semi-transparens** - Kablar är delvis genomskinliga (alpha 180)
- **Highlight på hover** - Röd glow-effekt från theme().colors.accent_red
- **Dragging-kablar** - Mindre sag (5%) för responsiv känsla
- **Auto-Layout** - Ny knapp "📐 Auto Layout" som organiserar moduler
  - Vänster→höger baserat på signalflöde (BFS från sources)
  - Envelopes/LFOs placeras under huvudsignalvägen
  - Konfigurerbara avstånd via LayoutConfig

---

## [0.33.18] - 2025-12-08
### Fixed - Parameter Routing for Arbitrary Modules
- **SetModuleParameter** används nu för alla voice-moduler istället för SetVoiceParameter
- Fixar parameter-routing för moduler utanför PolyModule enum (env-3, amp-2, sub-1, nse-1, kbp-1, etc)
- **Grand Piano patch** fungerar nu korrekt med alla 3 envelopes
- Tog bort redundant get_voice_module_for_param funktion
- Rensade oanvända imports (PolyModule, Param)

---

## [0.33.17] - 2025-12-08
### Changed - Physical Modeling Cleanup
- **Removed** StringResonator, ResonatorBank, VelocityMapper (fungerade inte korrekt)
- **KeyboardPanner** - Not-baserad stereopanorering nu registrerad i GUI-menyn
- **BodyResonance** - Resonanskropp-simulering nu registrerad i GUI-menyn
- **MechanicalNoise** - Mekaniska ljud nu registrerad i GUI-menyn
- **Grand Piano patch** - Uppdaterad med KeyboardPanner för stereo-imaging
- **Physical-menyn** i modulpaletten med de 3 fungerade modulerna

---

## [0.33.16] - 2025-12-08
### Added - Physical Modeling Modules
- **StringResonator** - Karplus-Strong string synthesis med inharmonicitet och dämpning
- **ResonatorBank** - Sympatisk resonans med 1-12 avstämbara strängar
- **KeyboardPanner** - Not-baserad stereopanorering för piano-liknande stereo
- **BodyResonance** - Resonanskropp-simulering (soundboard)
- **VelocityMapper** - Velocity-kurvor (Linear, Soft, Hard, S-Curve, Fixed)
- **MechanicalNoise** - Mekaniska ljud (tangent ner/upp, pedal, hammare)
- **ModuleCategory::PhysicalModeling** - Ny kategori för modulpalett

---

## [0.33.15] - 2025-12-08
### Added - Keyboard Splitting & MIDI Learn
- **KeyRange** - Ny typ för att definiera vilka noter ett instrument svarar på (keyboard splitting)
- **LearnState** - State machine för MIDI learn (Idle, WaitingForLowNote, WaitingForHighNote)
- **Transpose** - Semitone-offset per instrument (-24 till +24)
- **Instrument Rack UI** - Ny rad med Range-visning, Learn-knapp, Full-knapp, Transpose-kontroll
- **KeyRangeLearned event** - Engine skickar event till GUI när range lärs in
- **note_on/note_off** - Kollar key_range och applicerar transpose

---

## [0.33.14] - 2025-12-08
### Improved - GUI Styling & Theme Consistency
- **Master FX Sliders** - Bättre synlighet med mörkare bakgrund och tydlig kontrast
- **WidgetStyle** - Nya fält: knob_arc_segments, slider_rail_height, slider_handle_radius
- **knob.rs** - Använder nu theme().style istället för hårdkodade värden
- **meter.rs** - Använder nu theme().style istället för hårdkodade värden

---

## [0.33.13] - 2025-12-08
### Added - Attenuverters for CV Inputs
- **Filter CutoffMod** - Ny "CV Amt" parameter (-1.0 till +1.0) för cutoff CV
- **Oscillator FmAmount** - Ny "FM Amt" parameter (-1.0 till +1.0) för FM input
- Tog bort MIDI debug-spam

---

## [0.33.12] - 2025-12-08
### Added - Theme System
- 8 färgteman: Dark, Light, Vintage, Neon, Studio, Dracula, Monokai, Solarized Dark
- Tema-väljare i Settings, WidgetStyle för konsistent styling

---

## [0.33.11] - 2025-12-08
### Changed - InputPorts Refactor
- `PolyModule::process()` använder `InputPorts` wrapper istället för HashMap
- Eliminerar HashMap-allokering per audio frame

---

## [0.33.10] - 2025-12-08
### Fixed - Realtime Audio Allocations
- Eliminerade `AudioBuffer::new()` i audio thread (~187 allok/sek)
- `Connection` använder `PortName` (Copy) istället för String

---

## [0.33.9] - 2025-12-08
### Added - Bypass-knappar
- Power-knapp (⏻) i varje moduls header, bypassade moduler dimmas till 40%

---

## [0.33.8] - 2025-12-05
### Added - Master FX Parameters
- Resizable sidopanel, fullständiga parameterkontroller för alla 8 effekttyper

---

## [0.33.7] - 2025-12-05
### Added - Master FX Sidebar
- Kollapsbar effektlista i sidopanelen med bypass/remove per effekt

---

## [0.33.6] - 2025-12-05
### Added - Mixer & Master Bus
- Solo-knapp, global master effects chain, soft clipper per instrument

---

## [0.33.5] - 2025-12-05
### Fixed - Eliminated unwrap/expect
- Fixade 112 Clippy-varningar för unwrap/expect i produktionskod

---

## [0.33.4] - 2025-12-05
### Maintenance
- Rensade 29 onödiga clippy allows, uppdaterade 15 beroenden

---

## [0.33.3] - 2025-12-05
### Refactored - Clippy Pedantic
- Konfigurerade ~70 pedantic/nursery lints för synth-lämpliga undantag

---

## [0.33.2] - 2025-12-05
### Refactored - Best Practices
- `#[must_use]` på transformationsmetoder, tog bort global `#![allow(dead_code)]`
- Konverterade error-typer till thiserror, reducerade unsafe från 5 till 1 block

---

## [0.33.1] - 2025-12-04
### Refactored - Idiomatic Iterators
- Konverterade for-loopar till iteratorer utanför hot path, behöll for-loopar i DSP

---

## [0.33.0] - 2025-12-04
### Added - Type System Extensions
- `PortName` interning för zero-allocation, `FilterState` DSP-metoder
- `Hertz`, `NormalizedValue`, `MidiChannel`, `BeatDivision` extensions

---

## [0.32.25] - 2025-12-04
### Fixed - Instrument Channel Isolation
- Standardinstrument använder CH1 istället för OMNI
- Real-time safe sequencer (pre-allokerad event buffer)

---

## [0.32.24] - 2025-12-04
### Refactored - DSP Type Hardening
- `FilterState` i Reverb/Phaser, `Hertz::to_tan_coeff()`, `Milliseconds::to_samples()`

---

## [0.32.23] - 2025-12-04
### Fixed - PatchEditor GUI ID Collision
- Window ID inkluderar instrument_id för unika GUI-identifierare
- LadderFilter och NoiseGenerator använder `FilterState`

---

## [0.32.22] - 2025-12-04
### Refactored - Total Type Hardening
- `MidiNote` för PolyModule/EngineCommand, `SamplePosition`/`SampleCount` i Voice
- `BufferIndex` i Flanger/Chorus, `NormalizedValue` i SequencerTrack

---

## [0.32.21] - 2025-12-04
### Refactored - Type Safety and Enums
- `ModuleType::is_voice_module()`, unified `EffectChain` med `ChainSlot`
- Data-bärande `VoiceState` enum (Idle/Active/Releasing/Stealing)

---

## [0.32.20] - 2025-12-04
### Refactored - Per-Instrument Effects
- Flyttade effekter från global MasterBus till per-instrument EffectChain

---

## [0.32.19] - 2025-12-04
### Refactored - Per-Instrument PatchEditor
- Varje instrument äger sin egen PatchEditor, patch-laddning per instrument

---

## [0.32.18] - 2025-12-04
### Refactored - Per-Instrument Voice Architecture
- `voice_graph` ägs av Instrument istället för SynthEngine

---

## [0.32.17] - 2025-12-03
### Refactored - Architectural Terminology
- Renamed: RackView→PatchEditor, VoiceModule→PolyModule, EffectModule→AudioEffect
- SynthPart→Instrument, EffectChain→MasterBus

---

## [0.32.16] - 2025-12-03
### Added - Part Manager UI
- Multi-instrument support med Part Manager panel, MIDI-kanal per part

---

## [0.32.15] - 2025-12-03
### Improved - Knob Widget
- Värde visas i knob-cirkeln, centraliserad formatering i ParameterUnit
- Custom "Share Tech Mono" font

---

## [0.32.14] - 2025-12-03
### Added - MIDI Input Support
- Hardware MIDI via midir med GUI port-väljare, velocity visualization
- Type-safe MIDI parsing, pitch bend, mod wheel, aftertouch

---

## [0.32.13] - 2025-12-02
### Fixed - Stereo Output Parameters
- ModuleCategory::Output tillagd i parameter change handling

---

## [0.32.12] - 2025-12-02
### Removed - Performance Panel
- Tog bort GUI-komponenten, behöll engine-kommandona för framtida MIDI

---

## [0.32.11] - 2025-12-02
### Fixed - Real-time Parameter Updates
- `SetModuleParameter` uppdaterar nu voice_template + alla aktiva voices
- Tog bort 29 ogiltiga oscilloskop-kopplingar från patches

---

## [0.32.10] - 2025-12-02
### Fixed - Critical Audio Routing
- StereoOutput klassificeras som voice module, partiella inputs fungerar
- `ClearAllModules` rensar voice_template

---

## [0.32.9] - 2025-12-02
### Added - Dynamic Module Routing
- Moduler routas automatiskt till rätt graf baserat på typ

---

## [0.32.8] - 2025-12-01
### Refactored - Unified Voice/Graph
- Voice äger ModuleGraph istället för hårdkodad modullista (~400 rader borttaget)

---

## [0.32.7] - 2025-12-01
### Refactored - Unified Param Architecture
- `Param` enum med inbakade typade värden, tog bort `TypedValue`

---

## [0.32.6] - 2025-11-30
### Fixed - Dropdown Parameter Sync
- Dropdown-handlers skickar rätt TypedValue-variant

---

## [0.32.5] - 2025-11-30
### Added - Domain Types for Effects
- `Ratio` (kompression), `BeatDivision` (tempo-sync), `VoiceCount`

---

## [0.32.4] - 2025-11-30
### Fixed - Waveform Selection
- GUI skickar TypedValue::Waveform, tog bort noise från Oscillator (använd NoiseGenerator)

---

## [0.32.3] - 2025-11-30
### Fixed - CV Modulation Drift
- LadderFilter, LFO, Oscillator använder effective values utan att modifiera parametrar

---

## [0.32.2] - 2025-11-30
### Fixed - GUI/Engine Sync
- Startup använder patch_bridge::load_patch() för synkronisering

---

## [0.32.1] - 2025-11-30
### Fixed - Ghost Sound Bug
- ClearAllModules inaktiverar parts

---

## [0.32.0] - 2025-11-30
### Added - Type Safety & GUI
- Type-safe public APIs med Hertz, Cents, Gain, NormalizedValue
- "New Patch" i File-menyn

---

## [0.31.0] - 2025-11-30
### Added - GUI Module Support
- SubOscillator och NoiseGenerator i module palette
- 3 nya example patches

---

## [0.30.0] - 2025-11-30
### Added - DSP Improvements
- Envelope curves (attack_curve, decay_curve, release_curve)
- SubOscillator modul (sine, square, -1/-2 oktav)
- NoiseGenerator modul (white, pink, brown, blue, violet)

---

## [0.29.0] - 2025-11-30
### Refactored - Modular Patch Structure
- 16 patches extraherade till individuella filer i src/patches/

---

## [0.28.1] - 2025-11-30
### Added - Performance Fixes
- Velocity mapping till engine, tempo sync för Delay och LFO
- Module connectivity visualization (Connected/Orphaned/Disconnected)

---

## [0.28.0] - 2025-11-30
### Added - Performance Panel
- Pitch bend (spring-back), mod wheel, velocity mapping knobs

---

## [0.27.0] - 2025-11-30
### Added - Enhanced Oscilloscope & LFO Tempo Sync
- Oscilloskop med waveform history, time division, trigger modes
- LFO tempo sync med beat divisions

---

## [0.26.0] - 2025-11-30
### Added - Engine Events & CPU Tracking
- EngineEvent system med prioriterad kanal
- CPU usage tracking per modul

---

## [0.25.0] - 2025-11-30
### Added - Effect Bypass & Master Volume
- Effect bypass per slot, master volume control

---

## [0.24.0] - 2025-11-29
### Refactored - Hub Architecture
- EventHub för GUI-engine kommunikation, ersatte direkt polling

---

## [0.23.0] - 2025-11-29
### Added - Visual States & Animations
- ModuleVisualState för visuell feedback, cable animations

---

## [0.22.0] - 2025-11-29
### Added - Sequencer Engine
- SequencerEngine med transport, looping, note events

---

## [0.21.0] - 2025-11-29
### Added - Sequencer Data Model
- Song, Pattern, Note, Track, Automation strukturer

---

## [0.20.0] - 2025-11-29
### Added - Effect Chain & Visualizers
- MasterBus med insert effects och visualizers

---

## [0.19.0] - 2025-11-29
### Added - Oscilloscope Widget
- Real-time waveform display i GUI

---

## [0.18.0] - 2025-11-29
### Added - Level Meter Widget
- VU-meter med peak hold och gradient

---

## [0.17.0] - 2025-11-29
### Added - Patch Save/Load
- JSON-baserat patch-format med ModuleBuilder API

---

## [0.16.0] - 2025-11-29
### Added - Voice Allocator
- Polyfoni med voice stealing, mono/poly modes

---

## [0.13.2] - 2025-11-29
### Fixed - Command Queue Overflow
- Ökade COMMAND_BUFFER_SIZE, DroppedModule wrapper

---

## [0.13.1] - 2025-11-29
### Added - Pink Noise & Linear FM
- Pink noise (Voss-McCartney), Linear FM mode, velocity sensitivity

---

## [0.13.0] - 2025-11-29
### Added - Math Oscillator
- 18 algoritmer: SineFM, TanChaos, SuperSaw, WaveFolder, Lorenz, KarplusStrong, etc.
- 6 nya example patches

---

## [0.12.0] - 2025-11-28
### Initial Release
- Moduler: Oscillator, Filter, Envelope, LFO, Amplifier, Mixer
- Effekter: Delay, Reverb, Distortion, Chorus, Phaser, Flanger, Compressor, EQ
- 10 example patches, piano keyboard, visual cable connections
