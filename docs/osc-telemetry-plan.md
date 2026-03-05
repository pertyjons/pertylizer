# OSC Telemetry & Bevy Visualizer — Implementation Plan

## Status

| Phase | Status | Description |
|-------|--------|-------------|
| Phase 1 | **Complete** | `synth_osc` crate — sender thread, event ring buffer, FFT, OSC encoding |
| Phase 2 | **Complete** | Bevy visualizer — FFT bars, RMS light, note flash, orbital camera, bloom, beat pulse |
| Phase 3 | **In progress** | Polish & extend — idle mode, additional telemetry, particle effects, effect system |
| Phase 4 | Future | Creative effects, per-instrument streams, video export |

## Overview

Pertylizer sends OSC telemetry over UDP to `127.0.0.1:9000`. A separate Bevy 3D visualizer
(`visualizer/`) receives and renders the data. OSC is enabled by default (`--no-osc` to disable).

The protocol is versioned. The sender emits `/synth/meta` on startup and periodically, and
includes a monotonic sequence number so the receiver can drop out-of-order packets.

---

## Part 1: Synth Side (`synth_osc` crate) — COMPLETE

### 1.1 Architecture

```
Audio Thread (real-time)          Shared State             OSC Sender Thread
┌──────────────────────┐    ┌──────────────────────┐    ┌─────────────────────┐
│ process() callback   │    │ EngineState.meters   │    │ Polls shared state  │
│  ├─ update_meters()──┼───→│  peak_l/r (atomic)   │←───┤  at ~30 Hz          │
│  │                   │    │  rms_l/r  (atomic)   │    │                     │
│  ├─ spectrum_analyzer│    ├──────────────────────┤    │ Reads VisBuf snap-  │
│  │  └─ run_fft()─────┼───→│ VisualizationBuffer  │←───┤  shots for FFT      │
│  │                   │    │  magnitude_buf (lock) │    │                     │
│  ├─ push EngineEvent─┼──┐ ├──────────────────────┤    │ Consumes events for │
│  │  NoteTriggered    │  │ │ TransportState       │←───┤  note on/off        │
│  │  NoteReleased     │  │ │  playing, tempo, pos │    │                     │
│  │  VoiceCount       │  │ └──────────────────────┘    │ Encodes to OSC via  │
│  │  CpuUsage         │  │                             │  `rosc` and sends   │
│  └───────────────────┘  │  OSC Event Ring Buffer      │  via UdpSocket      │
│                          └→ ringbuf (lock-free)   ←───┤                     │
└──────────────────────┘     (single consumer)          └─────────────────────┘
```

Zero allocations in the audio thread. The only change is one additional `try_push()` call
per event to the OSC ring buffer.

### 1.2 Implemented OSC Address Map

| Address                     | Type          | Rate   | Source                    |
|-----------------------------|---------------|--------|---------------------------|
| `/synth/meta`               | `[i, f, f]`   | ~1 Hz + start | OSC sender        |
| `/synth/meta/seq`           | `[i]`         | Bundle | OSC sender                |
| `/synth/audio/rms`          | `[f, f]`      | ~30 Hz | `MeterState` atomics      |
| `/synth/audio/peak`         | `[f, f]`      | ~30 Hz | `MeterState` atomics      |
| `/synth/audio/fft`          | `[f * 128]`   | ~30 Hz | `VisualizationBuffer`     |
| `/synth/event/note_on`      | `[i, i, i]`   | Event  | `EngineEvent` ring buffer |
| `/synth/event/note_off`     | `[i, i]`      | Event  | `EngineEvent` ring buffer |
| `/synth/transport/state`    | `[i, f, f]`   | ~30 Hz | `SharedTransportState`    |
| `/synth/engine/voice_count` | `[i]`         | Event  | `EngineEvent` ring buffer |
| `/synth/engine/cpu`         | `[f]`         | ~1 Hz  | `EngineEvent` ring buffer |
| `/synth/audio/centroid`     | `[f]`         | ~30 Hz | FFT (spectral centroid Hz)  |
| `/synth/audio/flux`         | `[f]`         | ~30 Hz | FFT (onset detection)       |
| `/synth/transport/phase`    | `[f]`         | ~30 Hz | Beat phase 0..1             |
| `/synth/event/cc`           | `[i, f, i]`   | Event  | MIDI CC (cc, value, channel) |
| `/synth/engine/event_drops` | `[i]`         | ~30 Hz | Ring buffer drops (if > 0)  |
| `/synth/meta/fft_freqs`     | `[f * 128]`   | ~0.2 Hz | Band center frequencies Hz |
| `/viz/ping`                 | `[]`          | ~0.2 Hz | Client discovery beacon     |
| `/viz/pong`                 | `[]`          | ~0.5 Hz | Client presence reply       |

### 1.3 Crate Structure

```
crates/synth_osc/
├── Cargo.toml
└── src/
    ├── lib.rs            — Public API: OscTelemetry::new(), start(), stop()
    ├── addresses.rs      — OSC address constants
    ├── sender.rs         — Background thread: poll state, drain events, encode + send
    └── config.rs         — OscConfig: target_addr, target_port, update_rate_hz
```

### 1.4 Dependencies

```toml
rosc = "0.11.4"
ringbuf = { workspace = true }
synth_core = { path = "../synth_core" }
synth_engine = { path = "../synth_engine" }
```

---

## Part 2: Bevy Visualizer (`visualizer/`) — COMPLETE

### 2.1 Project Structure

```
visualizer/
├── Cargo.toml
└── src/
    ├── main.rs              — Bevy app entry point
    ├── osc_receiver.rs      — Non-blocking UDP listener + OSC parser
    ├── telemetry.rs         — SynthTelemetry resource
    └── visuals/
        ├── mod.rs           — Plugin, scene setup (ground, lights, camera + bloom)
        ├── fft_bars.rs      — 128 cubes driven by FFT bands (hue gradient, smooth lerp)
        ├── rms_light.rs     — Point light intensity tracks RMS level
        ├── note_flash.rs    — Emissive sphere flashes on note-on (hue = MIDI note)
        ├── camera.rs        — Orbital camera (slow rotation)
        └── beat_pulse.rs    — Ground glow + ambient light pulse on beats (frame-rate independent)
```

### 2.2 Dependencies

```toml
bevy = "0.18.1"
rosc = "0.11.4"
```

### 2.3 Visual Systems

| System | Description |
|--------|-------------|
| `fft_bars` | 128 cubes, Y-scale follows FFT magnitude, smooth lerp (fast attack, slow decay) |
| `rms_light` | Point light intensity = mono RMS × 200k lumens |
| `note_flash` | Emissive sphere, hue from MIDI note, brightness from velocity, exponential decay |
| `beat_pulse` | Detects beat crossings via `beat_position`, stronger on downbeats, delta-time decay |
| `camera::orbit` | Slow Y-axis rotation around scene center |
| `Bloom` | HDR bloom on camera (intensity 0.3, low-frequency boost) |

---

## Phase 3: Polish & Extend (In Progress)

See `TODO.md` Priority 0 for the full task list.

### 3.1 OSC Idle Mode — COMPLETE
- `/viz/ping`↔`/viz/pong` handshake with 5s timeout
- Skips FFT and full telemetry when idle, sends meta beacon only
- GUI status indicator (Off/Idle/Connected) in top bar

### 3.2 Additional Telemetry Streams — COMPLETE
All priority streams implemented (see address map above).

### 3.3 Visualizer Improvements
- Particle systems for note events
- Camera auto-movement synced to tempo
- "Waiting for signal" indicator
- Protocol version check from `/synth/meta`
- Effect rack with switchable visual layers

---

## Phase 4: Future Ideas

### Creative Effect Catalog

| Effect | Visual idea | Driven by | Technical approach |
|---|---|---|---|
| Spectral Cathedral | FFT bands form arches that breathe | FFT, RMS | Instanced arches + emissive |
| Harmonic Ribbons | Ribbons track pitch and glide | Note-on, pitch | Spline trails, hue = note |
| Chord Bloom | Chords trigger radial bursts | Note clusters | Particle bursts + radial expansion |
| Pulse Terrain | Landscape breathes with bass | Low FFT, RMS | Heightmap displacement |
| Spectral Origami | Folded planes open with harmonics | FFT, centroid | Mesh folding + shader |
| Ferrofluid Tendrils | Magnetic tendrils from bass | Low FFT | Curl noise + instanced strands |
| Neon Calligraphy | Notes draw glyph strokes | Note on/off, pitch | SDF strokes + bloom |
| Fractal Pulse | Recursive shapes synced to beat | Tempo, RMS | Fractal instancing |
| CPU Overdrive Core | Glowing core that spins and fractures under load | CPU Usage, Voice Count | Rotating core with noise displacement, color shifting to red on high CPU |
| Flux Supernova | Star that explodes on sudden spectral changes | Spectral Flux, RMS | Particle explosion / bloom flash triggered by flux spikes |
| Phase Rings | Concentric rings expanding with the beat phase | Beat Phase, Tempo | Torus instances scaling from 0 to max radius synced to `beat_phase` |
| Centroid Nebula | Particle cloud shifting color/shape based on brightness | Spectral Centroid, RMS | Compute shader / particle system where centroid Hz shifts color (warm to cool) and turbulence |
| Velocity Meteors | Meteors falling with size based on impact | Note-on, Velocity | Spheres with trail renderer falling from top, size/brightness mapped to velocity |

### Future Features
- Per-instrument OSC streams and per-track visual layers
- `/viz/` OSC control endpoints (effect select, param set, scene load)
- Effect switching: manual, energy-based auto, or cue-driven
- Visual preset morphing and crossfade timeline
- Recording/export: render video to file
- GPU FFT for higher-resolution spectra

### Proposed `/viz/` Control Endpoints

| Address | Type | Purpose |
|---|---|---|
| `/viz/ping` | `[]` | Health check; visualizer replies with `/viz/pong` |
| `/viz/pong` | `[]` | Health response |
| `/viz/effect/select` | `[s]` | Select effect by name |
| `/viz/effect/next` | `[]` | Next effect (cycle) |
| `/viz/effect/prev` | `[]` | Previous effect |
| `/viz/param/set` | `[s, f]` | Set visual parameter by name |
| `/viz/switch/mode` | `[s]` | Switching mode: manual, auto, cue |
| `/viz/meta/intensity` | `[f]` | Set global intensity (0–1) |

### Full Additional Telemetry Streams (Lower Priority)

| Address | Type | Rate | Description |
|---|---|---|---|
| `/synth/audio/rolloff` | `[f]` | ~30 Hz | Spectral rolloff (85% energy) |
| `/synth/audio/flatness` | `[f]` | ~30 Hz | Spectral flatness (noise vs tone) |
| `/synth/audio/transient` | `[f]` | Event | Onset strength for impacts |
| `/synth/audio/loudness` | `[f]` | ~10 Hz | LUFS-style loudness |
| `/synth/audio/crest` | `[f]` | ~10 Hz | Crest factor (peak/RMS) |
| `/synth/midi/pitchbend` | `[i, i]` | Event | Pitch bend (value, channel) |
| `/synth/midi/aftertouch` | `[i, i]` | Event | Channel aftertouch |
| `/synth/voice/positions` | `[f * N]` | ~10 Hz | XYZ of active voices |
| `/synth/voice/pitch` | `[f * N]` | ~10 Hz | Active voice MIDI pitches |
| `/synth/voice/energy` | `[f * N]` | ~10 Hz | Per-voice energy 0..1 |
| `/synth/awe/rt60` | `[f]` | ~10 Hz | Current reverb RT60 |
| `/synth/awe/room_dims` | `[f, f, f]` | Event | Room dimensions |
| `/synth/awe/source_pos` | `[f, f, f]` | ~10 Hz | Source position XYZ |
| `/synth/engine/latency` | `[f]` | ~1 Hz | Output latency (ms) |
| `/synth/engine/xruns` | `[i]` | ~1 Hz | Underrun/overrun count |

---

## Bandwidth Estimate

At 30 Hz update rate (current implementation):

| Message          | Size (bytes) | Rate   | Bandwidth   |
|------------------|-------------|--------|-------------|
| Meta             | ~64         | 1 Hz   | 0.06 KB/s   |
| Seq              | ~24         | 30 Hz  | 0.7 KB/s    |
| RMS (2 floats)   | ~40         | 30 Hz  | 1.2 KB/s    |
| Peak (2 floats)  | ~40         | 30 Hz  | 1.2 KB/s    |
| FFT (128 floats) | ~560        | 30 Hz  | 16.8 KB/s   |
| Transport        | ~48         | 30 Hz  | 1.4 KB/s    |
| Note events      | ~40 each    | ~10/s  | 0.4 KB/s    |
| Voice count      | ~32         | ~5/s   | 0.16 KB/s   |
| CPU usage        | ~32         | 1 Hz   | 0.03 KB/s   |
| Centroid         | ~28         | 30 Hz  | 0.8 KB/s    |
| Flux             | ~28         | 30 Hz  | 0.8 KB/s    |
| Beat phase       | ~28         | 30 Hz  | 0.8 KB/s    |
| CC events        | ~44 each    | ~5/s   | 0.2 KB/s    |
| FFT freqs        | ~560        | 0.2 Hz | 0.1 KB/s    |
| **Total**        |             |        | **~25 KB/s** |

Well within localhost UDP capacity.
