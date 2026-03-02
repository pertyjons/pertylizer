# Pertylizer

> **Disclaimer:** This application is primarily AI-generated (built with Claude Code). The author takes no responsibility whatsoever for anything — use entirely at your own risk.

A modular audio synthesizer written in Rust with a real-time egui GUI, pattern sequencer, spatial audio engine, and MCP integration for AI-assisted sound design.

## Features

### Synthesis

- **35 voice modules** — 9 oscillator types (standard, wavetable, additive, granular, fractal, FM/math, sub, LA synth, vector), filters (ladder, SVF, biquad), envelopes, LFOs, MSEG, mod matrix, ring mod, and more
- **21 effects** — delay, BBD delay, reverb, shimmer reverb, reverse gate reverb, chorus, ensemble chorus, flanger, phaser, distortion, waveshaper, compressor, limiter, EQ, mid/side, convolver, phase vocoder, frequency shifter, granular FX, spectral blur, modal resonator
- **60 built-in patches** — from acid bass and grand piano to fractal cosmos and spectral freeze pad

### Highlights

- **Fractal Oscillator** — Weierstrass-function synthesis producing complex, evolving timbres
- **Granular Synthesis** — both as oscillator and real-time effect with grain cloud control
- **Spectral Processing** — phase vocoder, spectral blur, and partitioned convolution
- **Physical Modeling** — body resonance, mechanical noise, LA synth (bell/drum), modal resonator
- **Generative Sequencing** — Euclidean rhythm generator, Turing machine, random gates
- **AWE (Acoustic World Engine)** — physics-based spatial audio with room simulation, early reflections (image-source method), late reverb (FDN), room modes, per-voice 3D spatialization, and internal modulation LFOs
- **MCP Server (79 tools)** — full remote control via HTTP or stdio, enabling AI agents like Claude to build instruments, compose songs, tweak parameters, and play notes in real time

### Architecture

- **Modular patching** — connect modules freely via a DAG-based audio graph with cable visualization
- **Pattern sequencer** — pattern-based sequencing with song arrangement (960 PPQN)
- **Multitimbral** — per-instrument voice allocation and effect chains
- **Real-time safe** — lock-free audio thread with zero allocations, locks, or panics
- **MIDI** — hardware MIDI input with velocity, pitch bend, mod wheel, aftertouch

## Tech Stack

- **Language:** Rust 1.93+ (edition 2024)
- **Audio:** cpal (cross-platform I/O)
- **GUI:** egui/eframe with custom knobs, meters, scopes, and spectrum analyzer
- **MIDI:** midir
- **DSP:** PolyBLEP oscillators, SVF/biquad/ladder filters, FFT via realfft
- **MCP:** rmcp + axum (Streamable HTTP on port 9850)
- **Concurrency:** lock-free ringbuf, parking_lot

## Building & Running

```bash
# Build
cargo build

# Run with GUI (default)
cargo run

# Run with GUI + MCP server
cargo run --features mcp

# Run headless MCP server (stdio)
cargo run --features mcp -- --mcp

# Tests, lints, formatting
cargo test && cargo clippy --all-targets && cargo fmt --check
```

## Workspace Crates

| Crate | Description |
|-------|-------------|
| `synth_core` | Domain types, module traits, audio abstractions |
| `synth_dsp` | DSP primitives: oscillators, filters, delay lines, FFT |
| `synth_awe` | Acoustic World Engine — spatial audio & room simulation |
| `synth_sequencer` | Pattern and song sequencing |
| `synth_modules` | 35 voice modules and 21 effects |
| `synth_engine` | Audio engine: voice allocation, modular graph, mixing |
| `synth_mcp` | MCP server with 79 tools for AI agent integration |
| `pertylizer` | Main application: GUI, audio I/O, MIDI |
