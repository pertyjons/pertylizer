# Modular Synthesizer

A modular audio synthesis system written in Rust with a real-time egui GUI, pattern sequencer, and MCP integration for AI-assisted sound design.

## Features

- **20+ audio modules** — oscillators (additive, granular, wavetable), filters (ladder, SVF), envelopes, LFOs, sequencing modules (Euclidean, Turing machine), and more
- **13+ effects** — delay, reverb, chorus, flanger, distortion, compressor, limiter, EQ, convolver, phase vocoder
- **Modular patching** — connect modules freely via a DAG-based audio graph with cable visualization
- **Pattern sequencer** — pattern-based sequencing with song arrangement (960 PPQN)
- **Multitimbral** — per-instrument voice allocation and effect chains
- **Spatial audio (AWE)** — 3D room simulation with early reflections and wall absorption
- **MCP server** — 11 tools for remote control via HTTP or stdio, enabling AI agent integration
- **Real-time safe** — lock-free audio thread with no allocations, locks, or panics

## Tech Stack

- **Audio:** cpal (cross-platform I/O)
- **GUI:** egui/eframe with custom knobs, meters, scopes, and spectrum analyzer
- **MIDI:** midir
- **DSP:** PolyBLEP oscillators, SVF/biquad/ladder filters, FFT via realfft
- **MCP:** rmcp + axum (Streamable HTTP on port 9850)
- **Concurrency:** lock-free ringbuf, parking_lot

## Building & Running

Requires Rust 1.93+ (edition 2024).

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
| `synth_awe` | Spatial audio & room simulation |
| `synth_sequencer` | Pattern and song sequencing |
| `synth_modules` | Ready-to-use audio modules and effects |
| `synth_engine` | Audio engine: voice allocation, modular graph, mixing |
| `synth_mcp` | MCP server for AI agent integration |
| `modular_synth` | Main application: GUI, audio I/O, MIDI |
