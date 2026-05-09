# Pertylizer

[![Build & Test](https://github.com/pertyjons/pertylizer/actions/workflows/build.yml/badge.svg)](https://github.com/pertyjons/pertylizer/actions/workflows/build.yml)

> **Disclaimer:** This application is primarily AI-generated (built with Claude Code). The author takes no
> responsibility whatsoever for anything — use entirely at your own risk.

A modular audio synthesizer written in Rust with a real-time egui GUI, pattern sequencer, spatial audio engine, 3D
visualizer, and MCP integration for AI-assisted sound design.

## Screenshots

See [`screenshots/README.md`](screenshots/README.md) for a visual tour of the patch editor, sequencer, Acoustic World
Engine, and 3D visualizer.

## Features

### Synthesis

- **56 module types** — oscillators (standard, wavetable, additive, granular, fractal, FM/math, sub, LA synth, vector),
  filters (ladder, SVF, biquad), envelopes, LFOs, MSEG, mod matrix, ring mod, and more
- **21 effects** — delay, BBD delay, reverb, shimmer reverb, reverse gate reverb, chorus, ensemble chorus, flanger,
  phaser, distortion, waveshaper, compressor, limiter, EQ, mid/side, convolver, phase vocoder, frequency shifter,
  granular FX, spectral blur, modal resonator
- **60 built-in patches** — from acid bass and grand piano to fractal cosmos and spectral freeze pad

### Highlights

- **Fractal Oscillator** — Weierstrass-function synthesis producing complex, evolving timbres
- **Granular Synthesis** — both as oscillator and real-time effect with grain cloud control
- **Spectral Processing** — phase vocoder, spectral blur, and partitioned convolution
- **Physical Modeling** — body resonance, mechanical noise, LA synth (bell/drum), modal resonator
- **Generative Sequencing** — Euclidean rhythm generator, Turing machine, random gates
- **AWE (Acoustic World Engine)** — physics-based spatial audio with room simulation, early reflections (image-source
  method), late reverb (FDN), room modes, per-voice 3D spatialization, and internal modulation LFOs
- **MCP Server (~80 tools)** — full remote control via HTTP or stdio, enabling AI agents like Claude to build
  instruments, compose songs, tweak parameters, and play notes in real time
- **OSC Telemetry** — real-time spectrum, RMS, note events, and transport state streamed over UDP at 30 Hz (enabled by
  default, `--no-osc` to disable)

### Pattern Sequencer

- **Pattern-based sequencing** with song arrangement (960 PPQN)
- **Song repeat** — loops entire song; transport repeat button in toolbar
- **Pattern repeat** — loops individual pattern during playback; toggle in piano roll toolbar
- **Recording** — real-time MIDI recording with count-in, quantize grid, and overdub mode
- **Automation lanes** — per-pattern parameter automation

### Architecture

- **Modular patching** — connect modules freely via a DAG-based audio graph with cable visualization
- **Multitimbral** — per-instrument voice allocation and effect chains
- **Real-time safe** — lock-free audio thread with zero allocations, locks, or panics
- **MIDI** — hardware MIDI input with velocity, pitch bend, mod wheel, aftertouch

## Bevy 3D Visualizer

A separate application (`visualizer/`) that receives OSC telemetry and renders real-time 3D visuals driven by audio
analysis.

### Visual Effects (22 effects)

Effects are organized into layered scene slots:

| Slot           | Effects                                                                                                                                                   |
|----------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------|
| **Terrain**    | Base Floor, FFT Bars, Waveform Ring, Spectral Waterfall, Pulse Terrain, Spectral Origami                                                                  |
| **Hero**       | CPU Overdrive Core, Flux Supernova, Fractal Pulse, Ferrofluid Tendrils                                                                                    |
| **Ambient**    | Centroid Nebula, Spectral Cathedral, Reaction Diffusion                                                                                                   |
| **Transients** | Note Particles, Velocity Meteors, Phase Rings, Harmonic Ribbons, Chord Bloom, Neon Calligraphy, Instrument Cubes, Voronoi Shatter, FFT Terrain, Note Tree |

### Scene Presets (9 presets)

0. **Classic Pertylizer** — Spectral Waterfall + Note Particles
1. **The Matrix** — Pulse Terrain + CPU Overdrive + Centroid Nebula + Velocity Meteors
2. **Sacred Geometry** — Spectral Origami + Fractal Pulse + Spectral Cathedral + Chord Bloom
3. **Magnetic Storm** — Waveform Ring + Ferrofluid Tendrils + Centroid Nebula + Phase Rings + Harmonic Ribbons
4. **The Exploding Sun** — FFT Bars + Flux Supernova + Neon Calligraphy + Note Particles
5. **Metallic Orchestra** — Base Floor + Fractal Pulse + Centroid Nebula + Instrument Cubes
6. **Earthquake** — Voronoi Shatter + Ferrofluid Tendrils + Velocity Meteors + Note Particles
7. **Spectrum City** — FFT Terrain + Note Tree + Reaction Diffusion + Chord Bloom
8. **Living Forest** — Pulse Terrain + Note Tree + Centroid Nebula + Harmonic Ribbons + Note Particles

### Themes (8 themes)

Neon (default), Metal, Glass, Space, Synthwave, Ember, Arctic, Void

### Camera Modes (5 modes)

Orbit (default), Top-Down, Front, Fly-Through, Free Orbit

- Dolly-zoom triggers automatically on bass drops
- Auto-cut cycles through camera modes every ~20 seconds

### Visualizer Keyboard Shortcuts

| Key              | Action                                           |
|------------------|--------------------------------------------------|
| `Left` / `Right` | Previous / next scene preset                     |
| `Up` / `Down`    | Zoom in / out                                    |
| `R`              | Random scene (procedurally generated)            |
| `T` / `Shift+T`  | Next / previous theme                            |
| `C` / `Shift+C`  | Next / previous camera mode                      |
| `V`              | Toggle auto-cut (cycles camera modes every ~20s) |
| `F`              | Toggle fullscreen                                |
| `P`              | Save screenshot (PNG)                            |
| `H`              | Toggle debug HUD                                 |

### Debug HUD

Press `H` to show a semi-transparent overlay with:

- **Visuals** — active theme, camera mode, auto-cut state, scene composition (terrain/hero/ambient/transients)
- **Audio analysis** — RMS levels, peak levels, spectral centroid (Hz), spectral flux
- **Transport** — BPM, beat position, beat phase, voice count
- **Performance** — FPS, frame time (ms), CPU usage, event drops, data staleness
- **Technical** — FFT bin count, OSC protocol version

### Running the Visualizer

```bash
# Start the synth first (OSC telemetry enabled by default)
cargo run

# In another terminal, start the visualizer
cd visualizer && cargo run
```

## Synth Keyboard Shortcuts

| Key                     | Action                    |
|-------------------------|---------------------------|
| `Z` – `M`               | Play notes (C3–B3)        |
| `Q` – `I`               | Play notes (C4–C5)        |
| `2`, `3`, `5`, `6`, `7` | Black keys (sharps/flats) |
| `-` / `+`               | Shift octave down / up    |

## Tech Stack

- **Language:** Rust 1.93+ (edition 2024)
- **Audio:** cpal (cross-platform I/O)
- **GUI:** egui/eframe with custom knobs, meters, scopes, and spectrum analyzer
- **MIDI:** midir
- **DSP:** PolyBLEP oscillators, SVF/biquad/ladder filters, FFT via realfft
- **MCP:** rmcp + axum (Streamable HTTP on port 9850)
- **OSC:** rosc (Open Sound Control over UDP)
- **Visualizer:** Bevy 0.16 (3D rendering)
- **Concurrency:** lock-free ringbuf, parking_lot

## Building & Running

```bash
# Build
cargo build

# Run with GUI (MCP + OSC telemetry enabled by default)
cargo run

# Run without OSC telemetry
cargo run -- --no-osc

# Run headless (no GUI, MCP server on stdio)
cargo run -- --headless

# Tests, lints, formatting
cargo test && cargo clippy --all-targets && cargo fmt --check
```

## Workspace Crates

| Crate             | Description                                                  | 
|-------------------|--------------------------------------------------------------|
| `synth_core`      | Domain types, module traits, audio abstractions              |
| `synth_dsp`       | DSP primitives: oscillators, filters, delay lines, FFT       |
| `synth_awe`       | Acoustic World Engine — spatial audio & room simulation      |
| `synth_sequencer` | Pattern and song sequencing                                  |
| `synth_modules`   | 56 module types including 21 effects                         |
| `synth_engine`    | Audio engine: voice allocation, modular graph, mixing        |
| `synth_mcp`       | MCP server with ~80 tools for AI agent integration           |
| `synth_osc`       | OSC telemetry sender (spectrum, notes, transport over UDP)   | 
| `pertylizer`      | Main application: GUI, audio I/O, MIDI                       |
| `visualizer`      | Bevy 3D visualizer driven by OSC telemetry (separate binary) |
