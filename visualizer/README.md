# Pertylizer Visualizer

A real-time 3D visualizer built with Bevy that renders audio-reactive visuals driven by OSC telemetry from the Pertylizer synthesizer.

## Getting Started

```bash
# 1. Start the synth (OSC telemetry enabled by default on UDP port 9000)
cargo run

# 2. In another terminal, start the visualizer
cd visualizer && cargo run --release
```

The visualizer listens on UDP port 9000 for OSC data. When the synth is running, visuals react in real time to audio levels, FFT spectrum, note events, beat position, and more.

## How It Works

The synth streams telemetry at 30 Hz over OSC (Open Sound Control), including:

- **Audio analysis** — RMS levels, peak levels, 128-band FFT spectrum, spectral centroid, spectral flux
- **Note events** — note on/off with velocity, instrument ID, and instrument category
- **Transport** — playing state, tempo (BPM), beat position, beat phase
- **Engine stats** — voice count, CPU usage, event drops

The visualizer maps this data to 3D scenes composed of layered visual effects, with automatic theme coloring and camera movement.

An idle mode reduces update rate when no synth is connected — telemetry resumes automatically on reconnection via a ping/pong handshake.

## Visual Effects

Each scene is composed of up to four layered slots, each with one or more effects:

### Terrain (ground/base layer)

| Effect | Description |
|--------|-------------|
| **Base Floor** | Reflective ground plane |
| **FFT Bars** | Vertical bars driven by FFT bins with frequency-to-hue coloring (bass=red, highs=violet) |
| **Waveform Ring** | Circular ring shape modulated by audio waveform |
| **Spectral Waterfall** | Scrolling frequency waterfall display |
| **Pulse Terrain** | Ground mesh that pulses with bass energy |
| **Spectral Origami** | Folding geometric shapes driven by spectrum data |
| **Phase Rings** | Concentric rings modulated by beat phase |
| **Voronoi Shatter** | Ground cells that split and tumble on spectral flux spikes, reassembling during calm passages |
| **FFT Terrain** | 16x8 grid of pillars with height driven by FFT bins and frequency-to-hue coloring |

### Hero (centerpiece)

| Effect | Description |
|--------|-------------|
| **CPU Overdrive Core** | Glowing core that intensifies with CPU load |
| **Flux Supernova** | Expanding supernova triggered by spectral flux |
| **Fractal Pulse** | Pulsing fractal geometry driven by amplitude, boosted by voice count |
| **Ferrofluid Tendrils** | Organic tendrils that react to spectral data |
| **Note Tree** | L-system branching cylinders that grow on note events and wither during silence |

### Ambient (sky/atmosphere)

| Effect | Description |
|--------|-------------|
| **Centroid Nebula** | Nebula coloring driven by spectral centroid, energy scales with voice count |
| **Spectral Cathedral** | Cathedral-like arches with breathing animation boosted by velocity |
| **Reaction Diffusion** | 12x12 sphere grid simulating Gray-Scott reaction-diffusion patterns; centroid controls feed rate, RMS controls kill rate |

### Transients (event-driven)

| Effect | Description |
|--------|-------------|
| **Note Particles** | Particle bursts on note events, colored by instrument category |
| **Velocity Meteors** | Meteor streaks on high-velocity notes, colored by instrument |
| **Harmonic Ribbons** | Flowing ribbons that appear on note events |
| **Chord Bloom** | Blooming flower-like geometry on chord events |
| **Neon Calligraphy** | Glowing calligraphic strokes on note activity |
| **Instrument Cubes** | Cubes that spawn per instrument, colored by category |

### Always active

| Effect | Description |
|--------|-------------|
| **RMS Light** | Dynamic scene lighting driven by RMS levels; flashes on high-velocity notes |
| **Beat Pulse** | Subtle pulse synchronized to beat position |
| **Telemetry Color** | Per-instrument hue offsets (Drums=red, Bass=blue, Lead=gold, Pad=green, etc.) |

## Scene Presets

Cycle through presets with Left/Right arrow keys, or press R for a random scene.

| # | Name | Composition |
|---|------|-------------|
| 0 | **Classic Pertylizer** | Spectral Waterfall + Note Particles |
| 1 | **The Matrix** | Pulse Terrain + CPU Overdrive + Centroid Nebula + Velocity Meteors |
| 2 | **Sacred Geometry** | Spectral Origami + Fractal Pulse + Spectral Cathedral + Chord Bloom |
| 3 | **Magnetic Storm** | Waveform Ring + Ferrofluid Tendrils + Centroid Nebula + Phase Rings + Harmonic Ribbons |
| 4 | **The Exploding Sun** | FFT Bars + Flux Supernova + Neon Calligraphy + Note Particles |
| 5 | **Metallic Orchestra** | Base Floor + Fractal Pulse + Centroid Nebula + Instrument Cubes |
| 6 | **Earthquake** | Voronoi Shatter + Ferrofluid Tendrils + Velocity Meteors + Note Particles |
| 7 | **Spectrum City** | FFT Terrain + Note Tree + Reaction Diffusion + Chord Bloom |
| 8 | **Living Forest** | Pulse Terrain + Note Tree + Centroid Nebula + Harmonic Ribbons + Note Particles |

Random scenes (R) are procedurally generated: 80% chance of terrain, 70% hero, 50% ambient, and 1-3 random transients.

## Themes

Cycle with T / Shift+T. Each theme provides a distinct color palette applied to all effects.

| Theme | Style |
|-------|-------|
| **Neon** (default) | Bright neon colors on dark background |
| **Metal** | Industrial metallic tones |
| **Glass** | Translucent, cool-toned |
| **Space** | Deep space blues and purples |
| **Synthwave** | Retro pink/cyan/purple |
| **Ember** | Warm fire-like oranges and reds |
| **Arctic** | Cold whites and icy blues |
| **Void** | Dark, minimal, high contrast |

## Camera

Cycle with C / Shift+C.

| Mode | Description |
|------|-------------|
| **Orbit** (default) | Slow orbit around center, speed synced to tempo |
| **Top-Down** | Looking straight down at the scene |
| **Front** | Fixed front-facing view |
| **Fly-Through** | Camera moves forward along Z axis, looping back |
| **Free Orbit** | Orbit with different angle and height |

Additional camera features:
- **Zoom** — Up/Down arrow keys (range 5-60 units), camera height scales with distance
- **Dolly-zoom** — triggers automatically on bass drops (low FFT spikes widen FOV while camera pushes in)
- **Camera shake** — triggered by spectral flux spikes, decays exponentially
- **Auto-cut** — press V to enable; switches to a random camera mode every ~20 seconds

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `Left` / `Right` | Previous / next scene preset |
| `Up` / `Down` | Zoom in / out (hold) |
| `R` | Random scene (procedurally generated) |
| `T` / `Shift+T` | Next / previous theme |
| `C` / `Shift+C` | Next / previous camera mode |
| `V` | Toggle auto-cut (~20s camera rotation) |
| `F` | Toggle fullscreen |
| `P` | Save screenshot (PNG to working directory) |
| `H` | Toggle debug HUD |

## Debug HUD

Press H to show a semi-transparent overlay in the top-left corner displaying:

```
Pertylizer Visualizer
Theme: Neon   Camera: Orbit  Auto-cut: OFF
Terrain: SpectralWaterfall
Hero: -  Ambient: -
Transients: NoteParticles
FPS:   60.0   Frame:   16.7ms
--------------------------------
RMS:   0.12 /  0.11   Peak:   0.45 /  0.42
Cent:    1200 Hz     Flux:  0.042
BPM:   120.0          Beat:   3.2
Phase: 0.80           Voices:   4
CPU:    8.2%          Drops:     0
Stale:  0.0s          Seq:      142
FFT:  128 bins       Proto: v1
-----------------------
Shortcuts:
Left/Right  Effect prev/next
Up/Down     Zoom in/out
...
```

| Section | Details |
|---------|---------|
| **Scene** | Active theme, camera mode, auto-cut state, terrain/hero/ambient/transients |
| **Performance** | FPS, frame time |
| **Audio** | Stereo RMS and peak levels, spectral centroid (Hz), spectral flux |
| **Transport** | BPM, beat position, beat phase, active voice count |
| **Engine** | CPU usage, event drops, data staleness (seconds since last OSC packet) |
| **Protocol** | FFT bin count, OSC protocol version, sequence number |

## OSC Protocol

Communication uses OSC over UDP on port 9000 (configurable in code).

| Address | Data | Rate |
|---------|------|------|
| `/synth/audio/rms` | L, R levels (f32) | 30 Hz |
| `/synth/audio/peak` | L, R peak (f32) | 30 Hz |
| `/synth/audio/fft` | 128 normalized bands (f32) | 30 Hz |
| `/synth/audio/centroid` | Hz (f32) | 30 Hz |
| `/synth/audio/flux` | magnitude (f32) | 30 Hz |
| `/synth/event/note_on` | note, velocity, instrument, category | per event |
| `/synth/event/note_off` | note, instrument, category | per event |
| `/synth/event/cc` | cc number, value, channel | per event |
| `/synth/transport/state` | playing, tempo, beat position | 30 Hz |
| `/synth/transport/phase` | beat phase 0.0-1.0 | 30 Hz |
| `/synth/engine/voice_count` | count (i32) | 30 Hz |
| `/synth/engine/cpu` | percentage (f32) | 30 Hz |
| `/synth/meta` | protocol version, sample rate | every 5s |
| `/viz/ping` | (in bundle) | 30 Hz |
| `/viz/pong` | reply from visualizer | every 2s |
| `/viz/camera/mode` | mode name (string) | on demand |

## Tech Stack

- **Engine:** Bevy 0.16
- **OSC:** rosc (Open Sound Control)
- **Protocol:** synth_osc_protocol (shared crate with the synth)
- **Rendering:** Bevy PBR with StandardMaterial, dynamic meshes, and particle systems
