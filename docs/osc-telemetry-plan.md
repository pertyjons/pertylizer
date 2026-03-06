# OSC Telemetry & Bevy Visualizer — Implementation Plan

## Status

| Phase | Status | Description |
|-------|--------|-------------|
| Phase 1 | **Complete** | `synth_osc` crate — sender thread, event ring buffer, FFT, OSC encoding |
| Phase 2 | **Complete** | Bevy visualizer — FFT bars, RMS light, note flash, orbital camera, bloom, beat pulse |
| Phase 3 | **Complete** | Polish & extend — idle mode, additional telemetry, particle effects, effect system, performance optimization |
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

## Phase 3: Polish & Extend — COMPLETE

See `TODO.md` Priority 0 for the full task list.

### 3.1 OSC Idle Mode — COMPLETE
- `/viz/ping`↔`/viz/pong` handshake with 5s timeout
- Skips FFT and full telemetry when idle, sends meta beacon only
- GUI status indicator (Off/Idle/Connected) in top bar

### 3.2 Additional Telemetry Streams — COMPLETE
All priority streams implemented (see address map above).

### 3.3 Visualizer Improvements — COMPLETE
- Particle systems for note events
- Camera auto-movement synced to tempo
- "Waiting for signal" indicator
- Protocol version check from `/synth/meta`
- Effect rack with switchable visual layers (15 effects, Left/Right/R switching, fade-through-black crossfade)
- All 13 creative effects from Phase 4 catalog implemented

### 3.4 Visualizer Performance Optimization — COMPLETE
- Disabled shadow maps (eliminated 6 cube-face re-renders per frame)
- Shared hue-bucketed materials across all effects (16 shared instead of 128 unique per effect)
- Extracted `HueMaterialConfig` + helpers into `effects.rs` for DRY material setup/update
- Scale-based fade instead of per-entity material emissive mutation
- Removed `AlphaMode::Blend` (was disabling batching)
- Material updates gated by `FADE_EPSILON` threshold
- Reduced centroid_nebula particles (2000 → 500)
- Fixed velocity_meteors exponential shrink bug

---

## Phase 4: Future Ideas

### Creative Effect Catalog — ALL IMPLEMENTED

All 13 effects from the original catalog are implemented and available in the effect rack:

| Effect | Visual idea | Driven by | Status |
|---|---|---|---|
| Spectral Cathedral | FFT bands form arches that breathe | FFT, RMS | Done |
| Harmonic Ribbons | Ribbons track pitch and glide | Note-on, pitch | Done |
| Chord Bloom | Chords trigger radial bursts | Note clusters | Done |
| Pulse Terrain | Landscape breathes with bass | Low FFT, RMS | Done |
| Spectral Origami | Folded planes open with harmonics | FFT, centroid | Done |
| Ferrofluid Tendrils | Magnetic tendrils from bass | Low FFT | Done |
| Neon Calligraphy | Notes draw glyph strokes | Note on/off, pitch | Done |
| Fractal Pulse | Recursive shapes synced to beat | Tempo, RMS | Done |
| CPU Overdrive Core | Glowing core under load | CPU Usage, Voice Count | Done |
| Flux Supernova | Star explodes on spectral changes | Spectral Flux, RMS | Done |
| Phase Rings | Concentric rings with beat phase | Beat Phase, Tempo | Done |
| Centroid Nebula | Particle cloud shifting by brightness | Spectral Centroid, RMS | Done |
| Velocity Meteors | Meteors falling by velocity | Note-on, Velocity | Done |

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

---

## Code Review Findings (2026-03-06)

### Correctness / Bugs
- `visualizer/src/visuals/waveform_ring.rs`: `update()` forces `Visibility::Hidden` for active bars, so the effect never becomes visible. Should set `Visibility::Inherited` for active bins and only hide bars beyond `fft_bin_count`.
- `visualizer/src/visuals/centroid_nebula.rs`: `update_material()` uses a `Local<f32>` for `last_centroid` that is never updated, so hue never responds to the spectral centroid. Use `telemetry.centroid_hz` directly or store a shared smoothed centroid in a resource updated by `update()`.
- `visualizer/src/visuals/velocity_meteors.rs`: size scaling based on velocity is overwritten each frame (`transform.scale = Vec3::splat(life_pct)`), so velocity only affects the first frame. Store a base scale per meteor and multiply by `life_pct`.
- Transient effects freeze when deactivated because their update systems stop running, leaving stale entities hidden but alive (and they can “snap back” if the effect is re-enabled). Affects `harmonic_ribbons.rs`, `chord_bloom.rs`, `velocity_meteors.rs`, and `phase_rings.rs`. Consider always updating lifetimes even when inactive, or purge these entities when the effect is switched off.

### Performance / Optimization
- `visualizer/src/visuals/effects.rs`: `effect_active_or_fading()` makes *all* effect updates run during any crossfade (`fade > 0`). This can spike frame time on scene switches. Consider running updates only for effects in `active` or `pending` scenes, and keep crossfade-related material updates isolated.
- `visualizer/src/visuals/effects.rs`: `get_presets()` allocates a `Vec` every frame inside `input()`. Make presets a `const`/`lazy_static` or store in a resource to avoid per-frame allocation.
- `visualizer/src/visuals/centroid_nebula.rs`: uses `AlphaMode::Blend` for all 500 particles even though alpha isn’t animated. This disables batching and increases overdraw; use opaque materials unless actual transparency is needed.
- `visualizer/src/osc_receiver.rs`: UDP receive buffer is fixed at 8192 bytes. Large bundles with many note events could overflow and drop packets. Consider a larger buffer (e.g., 64 KB) to be safe.
- `visualizer/src/visuals/fft_bars.rs` and `visualizer/src/visuals/waveform_ring.rs`: bar smoothing uses a fixed per-frame lerp (frame-rate dependent). Use time-based smoothing (`1.0 - exp(-k*dt)`) for consistent motion.

### Visual / UX Improvements
- Many note-driven effects respond only to `last_note_on`. Using `pending_note_events` for `particles.rs` and `velocity_meteors.rs` would better reflect dense chords and improve visual richness.
- `visualizer/src/visuals/waiting_indicator.rs`: staleness is measured in frames, so indicator timing changes with FPS. Track staleness in seconds for consistent UX across machines.
- Consider adding subtle environment lighting or fog in `visualizer/src/visuals/mod.rs::setup_scene` (e.g., a dim directional rim light, haze, or sky gradient) to improve depth and reduce the “black void” look when effects fade.
