# TODO - Pertylizer (v0.224.0)

## Priority 0 — OSC & Visualizer: Phase 3 (Polish & Extend)

> Full plan: [osc-telemetry-plan.md](osc-telemetry-plan.md)

### 0.1 OSC idle mode (`synth_osc`) — DONE
- [x] Skip FFT computation and UDP sends when no visualizer is connected
- [x] `/viz/ping`↔`/viz/pong` handshake: sender includes `/viz/ping` in meta bundles, visualizer replies with `/viz/pong` every 2s
- [x] Idle timeout (5s default): when no pong received, skip FFT + full telemetry, send meta-only beacon
- [x] Automatic resume when client connects, with console log for state changes

### 0.1b OSC status indicator in top bar — DONE
- [x] Show OSC status icon + text in the top bar (like MCP indicator)
- [x] Three states: **Off** (red), **Idle** (dim, sending beacon but no client), **Connected** (green, client responding with `/viz/pong`)
- [x] Hover tooltip with status description

### 0.2 Additional OSC telemetry streams — DONE
- [x] `/synth/audio/centroid` — spectral centroid (brightness) for effect selection
- [x] `/synth/audio/flux` — spectral flux for onset/section detection
- [x] `/synth/transport/phase` — beat phase 0..1 within current beat
- [x] `/synth/event/cc` — MIDI CC events (cc, value, channel) including pitch bend (128) and aftertouch (129)
- [x] `/synth/engine/event_drops` — event ring buffer drop count (only sent when > 0)
- [x] `/synth/meta/fft_freqs` — 128 center frequencies for log-spaced bar positioning (sent with meta)

### 0.3 Visualizer improvements (`visualizer/`) — DONE
- [x] Particle systems for note events (golden-angle burst on note-on, gravity + fade, 512 particle cap)
- [x] Camera auto-movement synced to tempo (orbit speed scales with BPM, 120 BPM = baseline)
- [x] "Waiting for signal" indicator when no OSC data received (fades in after ~2s stale)
- [x] Protocol version check — warn once on mismatch with `/synth/meta`
- [x] Visualizer handles all Phase 3 telemetry streams (centroid, flux, phase, cc, drops, fft_freqs)
- [x] Per-particle material mutations (up to 512/frame) — replaced with shared hue-bucketed materials across all effects
- [x] Configurable FFT bin count (64/128/256) — `MAX_FFT_BANDS` = 256, `fft_bin_count` adapts dynamically to received OSC data
- [x] Extract shared OSC address constants — `synth_osc_protocol` crate with all address constants and `PROTOCOL_VERSION`, used by both `synth_osc` and visualizer
- [x] Replace `last_note_on: Option<(u8, u8, u32, u8)>` with a named `NoteOnEvent` struct (fields: `midi_note`, `velocity`, `instrument_id`, `category`)

### 0.4 Visualizer effect system — DONE
- [x] Effect rack with switchable visual layers (Left/Right arrows for prev/next, R for random)
- [x] 2 additional effect modes: Waveform Ring (circular FFT) and Spectral Waterfall (scrolling 3D spectrogram)
- [x] Fade-through-black crossfade between effects on switch
- [x] Spectral waterfall: replace 2048-entity grid with texture-based approach (single quad + custom shader) for fewer draw calls (Implemented via shared materials and Y-scale scaling instead)
- [x] Replace per-effect `if active != MyEffect { return }` guards with Bevy run conditions — `effect_active()` and `effect_active_or_fading()` run condition factories, applied via `.run_if()` on all 24 systems

### 0.4b Visualizer performance optimization — DONE
- [x] Disabled shadow maps on point light (was rendering 6 extra cube-face passes of the entire scene)
- [x] Shared hue-bucketed materials for all effects — reduces draw calls via Bevy batching (e.g., 128 unique materials → 16 shared buckets)
- [x] Extracted `HueMaterialConfig`, `create_hue_materials()`, `update_hue_materials_for_fade()` helpers into `effects.rs`
- [x] Eliminated per-entity material mutation — use transform scale for fade/visibility instead of emissive changes
- [x] Removed `AlphaMode::Blend` from phase_rings (was disabling instancing/batching)
- [x] Reduced centroid_nebula particle count from 2000 → 500
- [x] Material updates only on meaningful change (fade delta > `FADE_EPSILON`)
- [x] Fixed velocity_meteors exponential shrink bug (`transform.scale *= scale` → `transform.scale = Vec3::splat(life_pct)`)
- [x] Pre-allocated mesh resources for chord_bloom and harmonic_ribbons (avoid per-spawn mesh creation)

### 0.5 Utilize all telemetry data in effects
> Full plan: [telemetry-effects-plan.md](telemetry-effects-plan.md)
- [x] Audit every effect — map which telemetry fields each effect actually reads vs. ignores
- [x] Extend ThemeConfig/ThemeMaterialPolicy with telemetry-reactive parameters (centroid hue range, flux burst hue, beat pulse strength, peak flash hue, rms emissive scale)
- [x] Store CC/Pitch bend/Aftertouch in SynthTelemetry (was received but discarded)
- [x] Create shared telemetry_color helpers (centroid_to_hue, flux_emissive_boost, beat_pulse_factor, rms_to_emissive, peak_exceeds_threshold)
- [x] Use spectral centroid to shift hue/color temperature (warm low, cool high) in 16 effects — theme-aware hue range per theme
- [x] Use spectral flux for intensity spikes, burst triggers, and transition accents in 12 effects + rms_light + beat_pulse
- [x] Use beat phase for pulsing, scaling, rotation sync across rhythmic effects (fft_bars, fractal_pulse, spectral_cathedral, pulse_terrain)
- [x] Use velocity to control brightness, size, and spawn intensity (rms_light flash, spectral_cathedral breathing, pulse_terrain bass boost)
- [x] Use MIDI CC / pitch bend for continuous parameter modulation (harmonic_ribbons wave width, spectral_origami fold angle, neon_calligraphy stroke width)
- [x] Use voice count to scale visual density/complexity (centroid_nebula energy, particles burst size, fractal_pulse pulse amount)
- [x] Use CPU usage for visual stress indicators (cpu_overdrive_core — already implemented)
- [x] Use per-instrument note events for instrument-specific colors/layers (particles, velocity_meteors, harmonic_ribbons use category_hue_offset)
- [x] Use transport state (playing/stopped/position) for scene-level changes (pulse_terrain, fractal_pulse, spectral_origami slow down when stopped)
- [x] Use peak levels for transient-driven flashes and camera shake (rms_light flash, beat_pulse brightness spike)
- [x] Use FFT band energy for per-frequency color mapping (bass → red, mids → green, highs → blue) via nonlinear band_frequency_hue in fft_bars, spectral_waterfall, spectral_cathedral

### 0.6 Settings & control
- [ ] OSC enable/disable toggle in Pertylizer settings GUI
- [ ] `/viz/` OSC control endpoints (effect select, param set, scene load)
- [ ] Support connecting multiple OSC clients simultaneously (e.g., via `send_to` and active client tracking)

### 0.6 Shared dB conversion utility
- [ ] Extract `magnitude_to_normalized_db()` into `synth_core` or `synth_dsp` — inline `20.0 * x.log10()` + normalization repeated in 4+ locations

---

## Priority 1 — Foundation & Core Functionality

### 1.1 Undo/Redo — DONE
- [x] Implement undo/redo for sequencer operations (note add/delete/move, pattern edits)
- [x] Implement undo/redo for module operations (add, delete, move, parameter changes)
- [x] Implement undo/redo for connection operations (add, remove)
- [x] Keyboard shortcuts: Ctrl+Z / Ctrl+Shift+Z

### 1.2 Audio export — DONE
- [x] Render arrangement to WAV file (offline, faster-than-realtime)
- [x] Export dialog: file path, sample rate, bit depth, duration/range
- [x] Progress bar during render

### 1.3 Song save/load — DONE
- [x] Recent projects — remember last opened projects in settings, show in menu
- [x] Dirty state tracking — warn on unsaved changes before loading or quitting

### 1.4 Copy/paste modules — DONE
- [x] Copy a module with its current parameters
- [x] Paste as a new instance with the same settings
- [x] Copy a selection of modules + their internal connections
- [x] Ctrl+C / Ctrl+V / Ctrl+D keyboard shortcuts
- [x] Edit menu items (Copy, Paste, Duplicate)

### 1.5 Settings expansion
- [ ] Add Browse button in Settings dialog to change patches directory

### 1.6 Template library
- [ ] Add patch template directory and `Save Patch as Template` action
- [ ] Add Patch Template browser to load patch templates
- [ ] Support optional `license` and `min_app_version` metadata in group templates

---

## Priority 2 — UI Structure & Layout

### 2.1 Effects section — visual separation from voice modules
- [ ] Add a visual divider between voice modules and effects in the grid
- [ ] Use a distinct background color/tint for the effects area
- [ ] Label the section clearly: "Master Effects" or "Effect Chain"

### 2.2 Module Groups — Phase 2–3
- [ ] Phase 2: Template variants (parameter presets with remap)
- [ ] Phase 3: Probes data pipeline (ringbuffers, audio-thread safe collection)
- [ ] Phase 3: Probe rendering (waveform/spectrum/meter) with PortType-based signal type
- [ ] Phase 3: Polyphony probes = sum of voices (mixdown)

---

## Priority 3 — Visual Polish

### 3.1 Improve module knobs
- [ ] Better visual design — gradient fill, shadow, tick marks, value tooltip
- [ ] Consistent sizing across module types
- [ ] Arc-style knobs with colored fill showing current value

### 3.2 Improve module ports
- [ ] Clearer port type distinction (audio vs control vs gate vs MIDI)
- [ ] Better hover feedback
- [ ] Colored rings matching cable colors, port labels on hover

---

## Priority 4 — AWE Improvements

Findings and concrete ideas: `docs/AWE-Improvement-Findings.md`.

### 4.1 Rework room visualization
- [ ] Redesign the 3D isometric room rendering
- [ ] Improve animations (sound rings, reflection paths)
- [ ] Better visual clarity for room shape and dimensions

### 4.2 Differentiate effects more clearly
- [ ] Each material/effect should have more distinct visual representation
- [ ] Color-coded zones, animated textures per material, spectral visualization

---

## Priority 5 — Future / Later

### 5.1 Redesign instrument list
- [ ] Tabbed interface, mixer-style vertical strips, or collapsible panels

### 5.2 MIDI learn
- [ ] Map MIDI CC to any module parameter via right-click → "MIDI Learn"
- [ ] Visual indicator on mapped parameters
- [ ] Save/load MIDI mappings with patch or settings

### 5.3 Module presets
- [ ] Save/load parameter presets per module type (not the whole patch)
- [ ] Preset browser in module context menu or header
- [ ] Ship default presets for common module types

### 5.4 Polyphony settings
- [ ] Voice count configurable per instrument (GUI control)
- [ ] Voice stealing mode selection (oldest, quietest, none)
- [ ] Unison detune/spread controls

### 5.5 Future Visualizer Effects
- [x] Spectral Cathedral (FFT bands form arches that breathe, driven by FFT, RMS)
- [x] Harmonic Ribbons (Ribbons track pitch and glide, driven by Note-on, pitch)
- [x] Chord Bloom (Chords trigger radial bursts, driven by Note clusters)
- [x] Pulse Terrain (Landscape breathes with bass, driven by Low FFT, RMS)
- [x] Spectral Origami (Folded planes open with harmonics, driven by FFT, centroid)
- [x] Ferrofluid Tendrils (Magnetic tendrils from bass, driven by Low FFT)
- [x] Neon Calligraphy (Notes draw glyph strokes, driven by Note on/off, pitch)
- [x] Fractal Pulse (Recursive shapes synced to beat, driven by Tempo, RMS)
- [x] CPU Overdrive Core (Glowing core that spins and fractures under load, driven by CPU Usage, Voice Count)
- [x] Flux Supernova (Star that explodes on sudden spectral changes, driven by Spectral Flux, RMS)
- [x] Phase Rings (Concentric rings expanding with the beat phase, driven by Beat Phase, Tempo)
- [x] Centroid Nebula (Particle cloud shifting color/shape based on brightness, driven by Spectral Centroid, RMS)
- [x] Velocity Meteors (Meteors falling with size based on impact, driven by Note-on, Velocity)

### 5.6 Visualizer themes — DONE
- [x] Theme system — swap material palettes, lighting, and ground/sky across all effects
- [x] **Neon** — dark background, saturated emissive colors, heavy bloom
- [x] **Metal** — brushed steel materials, specular reflections, cool white lighting
- [x] **Glass** — translucent refractive materials, caustic-style highlights
- [x] **Space** — starfield skybox, deep blue/purple palette, nebula fog
- [x] **Synthwave** — grid floor, pink/cyan gradient sky, retro sun
- [x] **Ember** — warm orange/red palette, glowing particles, dark smoke fog
- [x] **Arctic** — icy blue/white palette, frosted materials, soft ambient light
- [x] **Void** — pure black background, minimal monochrome white, high contrast
- [x] Theme switching via keyboard shortcut (T / Shift+T)
- [ ] OSC `/viz/theme/select` control endpoint

### 5.7 Post-processing & shader effects
- [ ] Chromatic aberration — intensity scales with RMS level
- [ ] Glitch/distortion effect — triggered by CPU spikes or spectral flux
- [ ] Kaleidoscope mode — radial scene mirroring (configurable segment count)
- [ ] CRT/VHS filter — scanlines, color bleed, static noise
- [ ] Motion blur — strength synced to tempo

### 5.8 Camera modes
- [ ] Audio-reactive camera — shake on transients, dolly-zoom on bass drops
- [ ] Multiple camera presets (first-person fly-through, top-down, fixed angles, free orbit)
- [ ] Beat-synced camera cuts — VJ-style automatic camera switching on downbeats
- [ ] Camera mode switching via keyboard shortcut (C) or OSC `/viz/camera/select`

### 5.9 Multi-effect layering
- [ ] Show 2–3 effects simultaneously instead of one at a time
- [ ] Per-instrument visual layers — each instrument gets its own color/effect layer
- [ ] Blending modes between layers (additive, multiply, screen)
- [ ] Layer opacity control via OSC `/viz/layer/opacity`

### 5.10 Reactive environment
- [ ] Skybox that reacts to music — stars pulse with RMS, clouds move with tempo
- [ ] Reactive ground — ripples on note-on, cracks on bass hits
- [ ] Fog/mist density driven by reverb level or sustain
- [ ] Day/night cycle driven by song position
- [ ] Weather effects — rain on high spectral flux, lightning on transients

### 5.11 Generative geometry
- [ ] L-system trees — branches grow with incoming notes, wither during silence
- [ ] Voronoi patterns — cells split/shatter with spectral flux
- [ ] Reaction-diffusion patterns — driven by spectral centroid (warm ↔ cool)
- [ ] Fractal terrain — landscape deforms in real-time with FFT bands

### 5.12 Interaction & export
- [ ] Video recording — render to MP4 or image sequence
- [ ] Screenshot button (P key)
- [ ] OSC parameter tweaking — live control of intensity, speed, scale per effect
- [ ] Fullscreen toggle (F key)
- [ ] Debug HUD overlay — FPS, telemetry values, active effect, draw calls

### 5.13 Advanced simulations
- [ ] Swarm/flock simulation — particles flock or scatter based on dynamics (loud = scatter, quiet = flock)
- [ ] Cloth simulation — fabric that billows and ripples with FFT energy
- [ ] Text/typography — display song title, BPM, key in stylized 3D text
- [ ] AWE spatialization — if room data is available via OSC, visualize sound source position in 3D space
