# Modular Synthesizer

A flexible, modular audio synthesis system written in Rust.

## Version 0.12.0

## Features

- **Abstract audio backend** - Supports multiple audio APIs through a common trait interface
  - CPAL (default) - Cross-platform audio (WASAPI, CoreAudio, ALSA, JACK)
  - Null backend - For testing without audio hardware
  - Easy to add new backends (JACK, PortAudio, etc.)

- **Lock-free UI communication** - Real-time safe command queue
  - Commands from UI to audio thread via ring buffer
  - Events from audio thread to UI
  - Atomic shared state for meters and transport

- **Real-time safe audio processing**
  - No allocations in audio callback
  - No locks or blocking operations
  - CPU usage monitoring

- **Modular synthesis**
  - Oscillators with PolyBLEP anti-aliasing
  - State-variable filters (LP, HP, BP, Notch)
  - ADSR envelopes with curve control
  - LFOs with multiple waveforms
  - Amplifier/VCA with pan control
  - **StereoOutput** - Dedicated master output module (NEW in 0.12.0)

- **Voice management**
  - Polyphonic, mono, legato, unison modes
  - Voice stealing strategies
  - **Glide/Portamento** - Smooth pitch transitions (NEW in 0.12.0)
  - Unison detune

- **Effects**
  - Delay (mono/stereo/ping-pong)
  - Reverb (Freeverb-style)
  - Distortion (multiple types)
  - Chorus

## Requirements

- Rust 1.91+ (2024 edition)
- ALSA development files on Linux: `sudo apt install libasound2-dev`

## Quick Start

```bash
# Build and run
cargo run --release

# Run tests
cargo test

# Run benchmarks
cargo bench
```

## New in 0.12.0

### StereoOutput Module
Dedicated master output with:
- Master volume and pan controls
- Soft limiter to prevent clipping
- Peak metering

```rust
use modular_synth::modules::StereoOutput;

let mut output = StereoOutput::new();
output.set_master_level(0.8);
let peaks = output.get_peak_levels();
```

### Glide/Portamento
Smooth pitch transitions for mono/legato modes:

```rust
// Enable glide on voice allocator
allocator.set_glide_time(0.2); // 200ms

// Or per-voice
voice.set_glide_time(0.15);
voice.glide_to_note(72);
```

### Theme System Fix
Now uses `parking_lot::RwLock` for deadlock-free operation.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                          GUI                                 │
│  - Sends EngineCommands via lock-free queue                 │
│  - Reads meters/state via atomics                           │
└──────────────┬──────────────────────────┬───────────────────┘
               │ Commands                  │ State reads
               ▼                           │
┌──────────────────────────────────────────▼──────────────────┐
│                      SynthEngine                             │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │ CommandQueue│  │ ModuleGraph │  │ SharedState         │  │
│  │ (lock-free) │  │ (routing)   │  │ (atomic values)     │  │
│  └─────────────┘  └─────────────┘  └─────────────────────┘  │
└─────────────────────────┬───────────────────────────────────┘
                          │ AudioProcessor trait
┌─────────────────────────▼───────────────────────────────────┐
│                      AudioHost                               │
└─────────────────────────┬───────────────────────────────────┘
                          │ AudioBackend trait
             ┌────────────┼────────────┐
             ▼            ▼            ▼
        ┌─────────┐  ┌─────────┐  ┌─────────┐
        │  Cpal   │  │  Null   │  │ (future)│
        │ Backend │  │ Backend │  │  JACK   │
        └─────────┘  └─────────┘  └─────────┘
```

## Usage Example

```rust
use modular_synth::{audio, engine};

// Create the synth engine
let (engine, mut handle) = engine::SynthEngine::new();

// Create audio host with default backend
let mut host = audio::default_host()?;

// Start audio
let config = audio::StreamConfig::default();
host.start_output(None, &config, engine)?;

// Play a note
handle.note_on(60, 0.8); // Middle C, velocity 0.8

// Check meters
let (peak_l, peak_r) = handle.peak_meters();
println!("Peak: L={peak_l:.2} R={peak_r:.2}");

// Stop when done
host.stop()?;
```

## Demo Commands

When running the demo application:

| Command | Description |
|---------|-------------|
| `note <0-127> [velocity]` | Play a MIDI note |
| `off` | Stop all notes |
| `vol <0.0-1.0>` | Set master volume |
| `adsr <a> <d> <s> <r>` | Set envelope times (seconds) |
| `glide <seconds>` | Set portamento time |
| `meters` | Show peak/RMS levels |
| `cpu` | Show CPU usage |
| `latency` | Show audio latency |
| `quit` | Exit |

## Documentation

- [API Documentation](docs/API_0.12.0.md)
- [Improvement List](docs/IMPROVEMENT_LIST.md)
- [Type Safety Analysis](docs/TYPE_SAFETY_ANALYSIS.md)

## License

MIT
