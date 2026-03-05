# OSC Telemetry & Bevy Visualizer — Implementation Plan

## Overview

Add Open Sound Control (OSC) telemetry output to Pertylizer and build a separate Bevy 3D visualizer
that receives the data. The two projects communicate exclusively via UDP on `127.0.0.1:9000`.

The protocol is versioned. The sender emits a small `/synth/meta` message on startup and
periodically, and includes a monotonic sequence number so the receiver can drop out-of-order
packets. State telemetry (RMS/FFT/transport) runs at a steady rate; discrete events are drained
more frequently to reduce latency.

**Repositories:**
- `pertylizer` (this project) — OSC sender, new crate `synth_osc`
- `pertylizer-visualizer` (new repo) — Bevy OSC receiver + 3D graphics

---

## Part 1: Synth Side (`synth_osc` crate)

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

**Key design decision:** We do NOT touch the audio thread's hot path. Instead:

1. **Continuous data** (RMS, peak, FFT, transport) — the OSC sender thread polls the existing
   shared atomic state (`EngineState.meters`, `VisualizationBuffer`, `SharedTransportState`)
   at `update_rate_hz` (~30 Hz). This is exactly how the GUI already works. Zero changes to the
   audio thread.

2. **Discrete events** (note on/off, voice count, CPU) — we add a second optional
   `ringbuf::HeapRb<EngineEvent>` consumer in the engine. The audio thread pushes to both
   the GUI ring buffer and the OSC ring buffer (if enabled). The OSC sender thread drains
   this buffer at `event_rate_hz` (~120 Hz) or whenever events are pending.

3. **Ordering** — every OSC bundle includes `/synth/meta/seq` with a monotonic sequence number.
   The receiver keeps the latest seq and ignores older packets.

This approach keeps the audio thread allocation-free and lock-free. The only change in the
audio thread is one additional `try_push()` call per event — the same pattern already used
for the GUI event buffer.

### 1.2 OSC Address Map

| Address                     | Type          | Rate   | Source                    |
|-----------------------------|---------------|--------|---------------------------|
| `/synth/meta`               | `[i, f, i, i, i, f, i]` | ~1 Hz + start | OSC sender |
| `/synth/meta/fft_freqs`     | `[f * 128]`   | On start + change | OSC sender |
| `/synth/meta/seq`           | `[i]`         | Bundle | OSC sender |
| `/synth/audio/rms`          | `[f, f]`      | ~30 Hz | `MeterState` atomics      |
| `/synth/audio/peak`         | `[f, f]`      | ~30 Hz | `MeterState` atomics      |
| `/synth/audio/fft`          | `[f * 128]`   | ~30 Hz | `VisualizationBuffer`     |
| `/synth/event/note_on`      | `[i, i, i, i]`| Event  | `EngineEvent` ring buffer |
| `/synth/event/note_off`     | `[i, i, i]`   | Event  | `EngineEvent` ring buffer |
| `/synth/transport/state`    | `[i, f, f]`   | ~30 Hz | `SharedTransportState`    |
| `/synth/engine/voice_count` | `[i]`         | Event  | `EngineEvent` ring buffer |
| `/synth/engine/cpu`         | `[f]`         | ~1 Hz  | `EngineEvent` ring buffer |
| `/synth/engine/event_drops` | `[i]`         | ~1 Hz  | OSC sender                |

**Argument details:**
- `/synth/meta` — `(protocol_version: i32, sample_rate: f32, block_size: i32, channels: i32, fft_bins: i32, update_rate_hz: f32, features: i32)`
- `/synth/meta/fft_freqs` — 128 floats, center frequency in Hz for each FFT band
- `/synth/meta/seq` — `(seq: i32)` monotonic per bundle, used for ordering
- `/synth/audio/rms` — `(left: f32, right: f32)` linear amplitude 0.0–1.0+
- `/synth/audio/peak` — `(left: f32, right: f32)` linear amplitude
- `/synth/audio/fft` — 128 floats, normalized 0.0–1.0 (downsampled from 1025 FFT bins)
- `/synth/event/note_on` — `(midi_note: i32, velocity: i32, channel: i32, voice_id: i32)`
- `/synth/event/note_off` — `(midi_note: i32, channel: i32, voice_id: i32)`
- `/synth/transport/state` — `(playing: i32, tempo_bpm: f32, beat_position: f32)`
- `/synth/engine/voice_count` — `(count: i32)`
- `/synth/engine/cpu` — `(percent: f32)` audio-thread usage over the last 1s (0.0–100.0)
- `/synth/engine/event_drops` — `(count: i32)` total drops from OSC event ring buffer

**Features bitmask (proposed):**
- `1 << 0` = FFT enabled
- `1 << 1` = Event drops telemetry
- `1 << 2` = Transport state

### 1.2.1 Additional Telemetry Streams (Future / Optional)

These are ideas for richer visual control. Each entry includes a suggested OSC path and
what it represents. Most are low-rate (1–10 Hz) except where noted.

| Address | Type | Rate | Description |
|---|---|---|---|
| `/synth/audio/centroid` | `[f]` | ~30 Hz | Spectral centroid (brightness) 0..1 |
| `/synth/audio/rolloff` | `[f]` | ~30 Hz | Spectral rolloff (e.g. 85% energy) |
| `/synth/audio/flux` | `[f]` | ~30 Hz | Spectral flux for onset/section cues |
| `/synth/audio/flatness` | `[f]` | ~30 Hz | Spectral flatness (noise vs tone) |
| `/synth/audio/transient` | `[f]` | Event | Onset strength for sharp impacts |
| `/synth/audio/loudness` | `[f]` | ~10 Hz | LUFS-style loudness estimate |
| `/synth/audio/crest` | `[f]` | ~10 Hz | Crest factor (peak / RMS) |
| `/synth/transport/phase` | `[f]` | ~30 Hz | Beat phase 0..1 within current beat |
| `/synth/transport/bar` | `[i]` | ~10 Hz | Current bar index |
| `/synth/transport/tempo_conf` | `[f]` | ~1 Hz | Tempo confidence 0..1 |
| `/synth/transport/section` | `[s]` | Event | Section label (verse, build, drop) |
| `/synth/midi/cc` | `[i, i, i]` | Event | `(cc, value, channel)` for MIDI CC |
| `/synth/midi/pitchbend` | `[i, i]` | Event | `(value, channel)` pitch bend |
| `/synth/midi/aftertouch` | `[i, i]` | Event | `(value, channel)` channel AT |
| `/synth/voice/active` | `[i]` | ~10 Hz | Active voice count (alt stream) |
| `/synth/voice/positions` | `[f * N]` | ~10 Hz | Flattened XYZ of active voices |
| `/synth/voice/pitch` | `[f * N]` | ~10 Hz | Active voice MIDI pitches |
| `/synth/voice/energy` | `[f * N]` | ~10 Hz | Per-voice energy 0..1 |
| `/synth/patch/name` | `[s]` | Event | Current patch name |
| `/synth/patch/id` | `[s]` | Event | Stable patch ID or hash |
| `/synth/patch/changed` | `[s]` | Event | Patch change reason (load, save) |
| `/synth/module/added` | `[s]` | Event | Module type added to graph |
| `/synth/module/removed` | `[s]` | Event | Module type removed |
| `/synth/cable/added` | `[s, s]` | Event | `(from, to)` module port names |
| `/synth/cable/removed` | `[s, s]` | Event | `(from, to)` module port names |
| `/synth/param/changed` | `[s, f]` | Event | Param name + new value |
| `/synth/param/macro` | `[s, f]` | ~30 Hz | Macro value by name |
| `/synth/seq/step` | `[i, i]` | Event | `(track, step)` for sequencer |
| `/synth/seq/gate` | `[i, i]` | Event | `(track, gate)` trigger pulse |
| `/synth/key/estimate` | `[s, f]` | ~1 Hz | Key estimate + confidence |
| `/synth/chord/estimate` | `[s, f]` | ~1 Hz | Chord estimate + confidence |
| `/synth/harmony/tension` | `[f]` | ~10 Hz | Harmonic tension 0..1 |
| `/synth/awe/rt60` | `[f]` | ~10 Hz | Current reverb RT60 (s) |
| `/synth/awe/early_energy` | `[f]` | ~10 Hz | Early reflection energy |
| `/synth/awe/late_energy` | `[f]` | ~10 Hz | Late tail energy |
| `/synth/awe/material` | `[s]` | Event | Current material name |
| `/synth/awe/room_dims` | `[f, f, f]` | Event | Room length/width/height |
| `/synth/awe/source_pos` | `[f, f, f]` | ~10 Hz | Source position XYZ |
| `/synth/awe/listener_pos` | `[f, f, f]` | ~10 Hz | Listener position XYZ |
| `/synth/engine/latency` | `[f]` | ~1 Hz | Total output latency (ms) |
| `/synth/engine/xruns` | `[i]` | ~1 Hz | Underrun/overrun count |

**Parameter details for additional streams**

| Address | Parameters (meaning / range) |
|---|---|
| `/synth/audio/centroid` | `centroid_norm` (0.0–1.0, 0 = low, 1 = Nyquist) |
| `/synth/audio/rolloff` | `rolloff_hz` (Hz where ~85% energy is below) |
| `/synth/audio/flux` | `flux_norm` (0.0–1.0 spectral change per frame) |
| `/synth/audio/flatness` | `flatness` (0.0–1.0, tonal → noisy) |
| `/synth/audio/transient` | `onset_strength` (0.0–1.0) |
| `/synth/audio/loudness` | `lufs` (approx LUFS, typically -60..0) |
| `/synth/audio/crest` | `crest_ratio` (linear peak/RMS, >= 1.0) |
| `/synth/transport/phase` | `beat_phase` (0.0–1.0) |
| `/synth/transport/bar` | `bar_index` (integer, 0-based) |
| `/synth/transport/tempo_conf` | `confidence` (0.0–1.0) |
| `/synth/transport/section` | `section_label` (string) |
| `/synth/midi/cc` | `cc` (0–127), `value` (0–127), `channel` (0–15) |
| `/synth/midi/pitchbend` | `value` (0–16383), `channel` (0–15) |
| `/synth/midi/aftertouch` | `value` (0–127), `channel` (0–15) |
| `/synth/voice/active` | `count` (active voices) |
| `/synth/voice/positions` | `x1,y1,z1,...` meters for each active voice (3*N floats) |
| `/synth/voice/pitch` | `pitch1..N` (MIDI note as float, supports microtonal) |
| `/synth/voice/energy` | `energy1..N` (0.0–1.0 per voice) |
| `/synth/patch/name` | `name` (string) |
| `/synth/patch/id` | `id` (string, stable hash or UUID) |
| `/synth/patch/changed` | `reason` (string, e.g. load, save, init) |
| `/synth/module/added` | `module_type` (string) |
| `/synth/module/removed` | `module_type` (string) |
| `/synth/cable/added` | `from`, `to` (strings like `module_id:port`) |
| `/synth/cable/removed` | `from`, `to` (strings like `module_id:port`) |
| `/synth/param/changed` | `param_name` (string), `value_norm` (0.0–1.0) |
| `/synth/param/macro` | `macro_name` (string), `value_norm` (0.0–1.0) |
| `/synth/seq/step` | `track` (int), `step` (int) |
| `/synth/seq/gate` | `track` (int), `gate` (0/1) |
| `/synth/key/estimate` | `key_name` (string), `confidence` (0.0–1.0) |
| `/synth/chord/estimate` | `chord_name` (string), `confidence` (0.0–1.0) |
| `/synth/harmony/tension` | `tension` (0.0–1.0) |
| `/synth/awe/rt60` | `seconds` (s) |
| `/synth/awe/early_energy` | `energy` (0.0–1.0) |
| `/synth/awe/late_energy` | `energy` (0.0–1.0) |
| `/synth/awe/material` | `material_name` (string) |
| `/synth/awe/room_dims` | `length, width, height` (meters) |
| `/synth/awe/source_pos` | `x, y, z` (meters) |
| `/synth/awe/listener_pos` | `x, y, z` (meters) |
| `/synth/engine/latency` | `latency_ms` (milliseconds) |
| `/synth/engine/xruns` | `count` (total underruns/overruns) |

### 1.3 New Files

```
crates/synth_osc/
├── Cargo.toml
└── src/
    ├── lib.rs            — Public API: OscTelemetry::new(), start(), stop()
    ├── addresses.rs      — OSC address constants
    ├── sender.rs         — Background thread: poll state, drain events, encode + send
    └── config.rs         — OscConfig: enabled, target_addr, target_port, update_rate_hz
```

### 1.4 Dependencies

```toml
[dependencies]
rosc = "0.10.1"                 # OSC encoding/decoding
ringbuf = { workspace = true }  # Lock-free event buffer
synth_core = { path = "../synth_core" }
synth_engine = { path = "../synth_engine" }
```

### 1.5 Public API

```rust
/// OSC telemetry sender configuration.
pub struct OscConfig {
    pub enabled: bool,
    pub target_addr: SocketAddr,     // default 127.0.0.1:9000
    pub update_rate_hz: f32,         // default 30.0
    pub event_rate_hz: f32,          // default 120.0
    pub send_fft: bool,              // default true (can disable to save bandwidth)
    pub send_meta_every_s: f32,      // default 1.0 (0 = only on start)
    pub send_fft_freqs: bool,        // default true
}

/// OSC telemetry sender. Owns the background thread.
pub struct OscTelemetry {
    config: OscConfig,
    thread_handle: Option<JoinHandle<()>>,
    stop_flag: Arc<AtomicBool>,
}

impl OscTelemetry {
    /// Create a new OSC telemetry sender.
    pub fn new(config: OscConfig) -> Self;

    /// Start the sender thread. Needs references to shared engine state.
    pub fn start(
        &mut self,
        engine_state: Arc<EngineState>,
        event_consumer: ringbuf::HeapCons<EngineEvent>,
        spectrum_buffer: Option<Arc<VisualizationBuffer>>,
    );

    /// Stop the sender thread gracefully.
    pub fn stop(&mut self);
}
```

### 1.6 Implementation Steps

**Step 1: Create the `synth_osc` crate skeleton**
- Add to workspace `Cargo.toml`
- Create `Cargo.toml`, `lib.rs`, `addresses.rs`, `config.rs`, `sender.rs`
- Define `OscConfig` and address constants

**Step 2: Implement the sender thread**
- Spawn a thread that loops at `event_rate_hz` (fast tick)
- Maintain timers: `next_state_tick`, `next_meta_tick`
- On each tick:
  - Always drain the event ring buffer and encode discrete events
  - If `now >= next_state_tick`, also include RMS/peak/FFT/transport in the same bundle
  - If `now >= next_meta_tick`, include `/synth/meta` (and `/synth/meta/fft_freqs` if enabled)
  - Prepend `/synth/meta/seq` to every bundle (monotonic `seq += 1`)
- Send the bundle via `UdpSocket` (fire-and-forget, ignore errors)
- If there are no messages to send, skip the bundle entirely

**Step 3: Add a second event ring buffer in `SynthEngine`**
- Add an `Option<ringbuf::HeapProd<EngineEvent>>` field to `SynthEngine`
- When OSC is enabled, the engine clones events to this second producer
- The consumer half is passed to `OscTelemetry::start()`
- Guard with `if let Some(ref mut prod) = self.osc_event_producer`
- Track `event_drop_count: AtomicU32` — increment when `try_push()` fails

**Step 4: Wire into the main application**
- In `pertylizer` crate, create `OscTelemetry` after engine starts
- Pass shared state references
- Add enable/disable toggle in settings (or just command-line flag initially)
- Stop on application exit

**Step 5: FFT data access**
- The master `SpectrumAnalyzer` already writes to a `VisualizationBuffer`
- Expose this buffer via `SynthEngine` so the OSC sender can read snapshots
- Downsample from 1025 complex bins to 128 bands (logarithmic grouping for perceptual accuracy)
- Compute and send `fft_freqs` (center frequency per band) in `/synth/meta/fft_freqs`

### 1.7 Critical Review — Synth Side

**What's good:**
- Zero allocation in audio thread — we only add one `try_push()` call
- Reuses all existing data infrastructure (atomics, VisualizationBuffer)
- Clean separation — `synth_osc` depends on `synth_engine` but not `pertylizer`
- Graceful degradation — if UDP send fails, we just drop the packet (fire-and-forget)
- Versioned protocol + `/synth/meta/seq` make receiver handling robust to changes

**Risks and mitigations:**
- **Risk:** VisualizationBuffer uses `parking_lot::Mutex` internally — the OSC thread could
  contend with the GUI thread. **Mitigation:** Both use `try_lock()`, so neither blocks.
  Worst case: one frame gets a stale snapshot.
- **Risk:** Event latency if we only poll at 30 Hz. **Mitigation:** Separate `event_rate_hz`
  loop (e.g. 120 Hz) or wake on pending events, so note flashes feel tight.
- **Risk:** FFT downsampling from 1025→128 bins could lose important frequency detail.
  **Mitigation:** Use logarithmic bin grouping (perceptual frequency scale), which matches
  how we hear. High frequencies have many bins averaged together, low frequencies keep detail.
- **Risk:** UDP packet size for FFT data: 128 floats × 4 bytes + OSC overhead ≈ 560 bytes.
  Well within the 1500-byte MTU. No fragmentation.
- **Risk:** Second event ring buffer doubles memory for events.
  **Mitigation:** Events are small (< 64 bytes each), buffer is 256 entries = ~16 KB. Negligible.
- **Risk:** Protocol changes break the visualizer silently.
  **Mitigation:** `/synth/meta` includes `protocol_version` and `features` bitmask; visualizer
  can warn on mismatch and degrade gracefully.

---

## Part 2: Bevy Visualizer (separate project)

### 2.1 Project Structure

```
pertylizer-visualizer/
├── Cargo.toml
├── src/
│   ├── main.rs           — Bevy app entry point
│   ├── osc_receiver.rs   — UDP listener system + OSC parser
│   ├── telemetry.rs      — SynthTelemetry resource
│   ├── visuals/
│   │   ├── mod.rs
│   │   ├── rms_light.rs  — RMS → light intensity system
│   │   ├── fft_bars.rs   — FFT → 3D bar scaling system
│   │   ├── note_flash.rs — Note events → particle/color bursts
│   │   └── camera.rs     — Orbital camera controller
│   └── setup.rs          — Scene setup (3D objects, lights, materials)
└── assets/               — Optional textures, models
```

### 2.2 Dependencies

```toml
[dependencies]
bevy = "0.18.1"
rosc = "0.10.1"
```

### 2.3 Core Resource

```rust
#[derive(Resource, Default)]
pub struct SynthTelemetry {
    /// Protocol metadata (from /synth/meta).
    pub protocol_version: i32,
    pub sample_rate: f32,
    pub block_size: i32,
    pub channels: i32,
    pub fft_bins: i32,
    pub update_rate_hz: f32,
    pub features: i32,
    /// FFT center frequencies (Hz) per band.
    pub fft_freqs: [f32; 128],
    /// Monotonic sequence number from /synth/meta/seq.
    pub seq: i32,
    /// Frames since last packet (stale detection).
    pub stale_frames: u32,
    /// RMS levels (left, right), linear amplitude.
    pub rms: [f32; 2],
    /// Peak levels (left, right), linear amplitude.
    pub peak: [f32; 2],
    /// FFT magnitude bands (128 bins), normalized 0.0–1.0.
    pub fft: [f32; 128],
    /// Most recent note-on event (MIDI note, velocity, channel, voice_id).
    pub last_note_on: Option<(u8, u8, u8, u32)>,
    /// Active voice count.
    pub voice_count: u32,
    /// Transport state.
    pub playing: bool,
    pub tempo: f32,
    pub beat_position: f32,
    /// CPU usage 0–100.
    pub cpu: f32,
    /// Total event drops reported by sender.
    pub event_drops: u32,
    /// Frame counter for note event decay.
    pub note_age_frames: u32,
}
```

### 2.4 OSC Receiver System

```rust
#[derive(Resource)]
pub struct OscSocket {
    socket: UdpSocket,
    buf: Vec<u8>,
}

fn setup_osc_socket(mut commands: Commands) {
    let socket = UdpSocket::bind("0.0.0.0:9000").expect("Failed to bind OSC port 9000");
    socket.set_nonblocking(true).expect("Failed to set non-blocking");
    commands.insert_resource(OscSocket {
        socket,
        buf: vec![0u8; 8192],
    });
}

fn receive_osc(mut socket: ResMut<OscSocket>, mut telemetry: ResMut<SynthTelemetry>) {
    // Drain all pending UDP packets (non-blocking)
    let mut received_any = false;
    loop {
        match socket.socket.recv(&mut socket.buf) {
            Ok(size) => {
                if let Ok((_, packet)) = rosc::decoder::decode_udp(&socket.buf[..size]) {
                    handle_packet(&packet, &mut telemetry);
                    received_any = true;
                }
            }
            Err(_) => break, // WouldBlock = no more data
        }
    }
    telemetry.stale_frames = if received_any { 0 } else { telemetry.stale_frames + 1 };
    // Age note events
    telemetry.note_age_frames += 1;
}

fn handle_packet(packet: &OscPacket, telemetry: &mut SynthTelemetry) {
    match packet {
        OscPacket::Message(msg) => handle_message(msg, telemetry),
        OscPacket::Bundle(bundle) => {
            // Optional sequence number for ordering
            if let Some(seq) = extract_seq(bundle) {
                if seq < telemetry.seq {
                    return; // drop out-of-order bundle
                }
                telemetry.seq = seq;
            }
            for p in &bundle.content {
                handle_packet(p, telemetry);
            }
        }
    }
}

fn handle_message(msg: &OscMessage, telemetry: &mut SynthTelemetry) {
    match msg.addr.as_str() {
        "/synth/meta" => { /* parse protocol_version, sample_rate, etc */ }
        "/synth/meta/fft_freqs" => { /* extract 128 floats → telemetry.fft_freqs */ }
        "/synth/meta/seq" => { /* extract i32 → telemetry.seq */ }
        "/synth/audio/rms" => { /* extract f32, f32 → telemetry.rms */ }
        "/synth/audio/peak" => { /* extract f32, f32 → telemetry.peak */ }
        "/synth/audio/fft" => { /* extract 128 floats → telemetry.fft */ }
        "/synth/event/note_on" => { /* extract i32, i32, i32, i32 → telemetry.last_note_on */ }
        "/synth/event/note_off" => { /* extract i32, i32, i32 → ... */ }
        "/synth/engine/event_drops" => { /* extract i32 → telemetry.event_drops */ }
        // ... etc
        _ => {}
    }
}
```

### 2.5 Visual Systems (examples)

**RMS Light:** A point light whose intensity tracks the RMS level.

```rust
#[derive(Component)]
struct RmsLight;

fn update_rms_light(
    telemetry: Res<SynthTelemetry>,
    mut query: Query<&mut PointLight, With<RmsLight>>,
) {
    let rms_mono = (telemetry.rms[0] + telemetry.rms[1]) * 0.5;
    for mut light in &mut query {
        light.intensity = rms_mono * 50_000.0; // Scale to Bevy lumens
    }
}
```

**FFT Bars:** 128 cubes whose Y-scale follows FFT band magnitude.

```rust
#[derive(Component)]
struct FftBar(usize); // bin index

fn update_fft_bars(
    telemetry: Res<SynthTelemetry>,
    mut query: Query<(&mut Transform, &FftBar)>,
) {
    for (mut transform, bar) in &mut query {
        let target = telemetry.fft[bar.0];
        // Smooth: lerp toward target for visual appeal
        let current = transform.scale.y;
        transform.scale.y = current + (target * 5.0 - current) * 0.15;
    }
}
```

**Note Flash:** Color burst on note-on events.

```rust
fn update_note_flash(
    telemetry: Res<SynthTelemetry>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    query: Query<&MeshMaterial3d<StandardMaterial>, With<NoteFlashSphere>>,
) {
    if let Some((note, velocity, _, _voice_id)) = telemetry.last_note_on {
        if telemetry.note_age_frames < 2 {
            // Map MIDI note to hue (0–127 → 0°–360°)
            let hue = (note as f32 / 127.0) * 360.0;
            let brightness = velocity as f32 / 127.0;
            // Update material emissive color
        }
    }
}
```

### 2.5.1 Creative Effect Catalog (Examples)

| Effect | Visual idea | Driven by | Technical approach |
|---|---|---|---|
| Spectral Cathedral | FFT bands form “cathedral arches” that breathe | FFT, RMS | Instanced arches, log-bin height + emissive |
| Harmonic Ribbons | Long ribbons track pitch and glide | Note-on, pitch, velocity | Spline trails, hue = note |
| Chord Bloom | Chords trigger radial bloom bursts | Note clusters, voice_count | Particle bursts + radial expansion |
| Voxel Room Modes | Room-mode pulses in a 3D voxel grid | Low FFT, modes | Voxel grid, per-band amplitude |
| Phase Lattice | Grid warps and ripples with spectral flux | FFT flux, RMS | Vertex displacement + noise |
| Fluid Interference | Water-like waves interfere and resonate | Peak, tempo/beat | 2D wave sim + normal map |
| Ferrofluid Tendrils | Magnetic tendrils from bass energy | Low FFT, resonance | Curl noise + instanced strands |
| Portal Tunnel | Reverb tail becomes a tunnel pull | Decay, RMS, tail | Radial extrusion + depth fog |
| Prism Burst | High-frequency bursts into prisms | High FFT, centroid | Procedural prisms + dispersion |
| Temporal Echo Glyphs | Echo glyphs trail behind notes | Note on/off, tempo | SDF glyphs with trail buffer |

### 2.5.2 Effect Switching (Manual + Automatic)

**Manual switching ideas**
- Effect rack with 1–3 active layers and crossfades.
- `Next/Prev` and `Random` with a cooldown to avoid rapid switching.
- Presets that store effect combos plus mapping ranges.
- Scene timeline for beat-synced swaps.

**Automatic switching ideas**
- Energy-based: RMS/peak over threshold → switch to high-energy set.
- Spectral-based: low centroid → bass-heavy effects; high centroid → prism effects.
- Section detection: spectral flux + beat position to detect “drop/verse”.
- Hysteresis and hold time to prevent chattering.
- Weighted random selection biased by current audio features.

### 2.5.3 Future Control From the Synth

- OSC control endpoints, for example:
  - `/viz/effect/select`
  - `/viz/effect/next`
  - `/viz/scene/load`
  - `/viz/param/<name>`
- Parameter mapping: map synth parameters to visual parameters with range + curve.
- Cue system: synth sends `cue_id` on patch/section changes for deterministic visuals.
- Optional metadata: `mood`, `intensity`, `density` to drive style selection.

**Proposed /viz OSC Address Map**

| Address | Type | Purpose |
|---|---|---|
| `/viz/ping` | `[]` | Health check; visualizer replies with `/viz/pong` |
| `/viz/pong` | `[]` | Health response |
| `/viz/effect/select` | `[s]` | Select effect by name or ID |
| `/viz/effect/next` | `[]` | Next effect (cycle) |
| `/viz/effect/prev` | `[]` | Previous effect |
| `/viz/effect/random` | `[]` | Random effect |
| `/viz/effect/enable` | `[s, i]` | Enable/disable effect by name (0/1) |
| `/viz/scene/load` | `[s]` | Load a scene/preset by name |
| `/viz/scene/save` | `[s]` | Save current scene/preset |
| `/viz/param/set` | `[s, f]` | Set parameter by name to value |
| `/viz/param/range` | `[s, f, f]` | Set param min/max mapping |
| `/viz/param/curve` | `[s, s]` | Set mapping curve (linear, exp, log) |
| `/viz/switch/mode` | `[s]` | Switching mode: manual, auto, cue |
| `/viz/switch/cooldown` | `[f]` | Minimum seconds between switches |
| `/viz/meta/mood` | `[s]` | Set mood tag (e.g. calm, intense) |
| `/viz/meta/intensity` | `[f]` | Set global intensity (0–1) |
| `/viz/meta/density` | `[f]` | Set density (0–1) |

### 2.6 Implementation Steps

**Step 1: `cargo init pertylizer-visualizer`**
- Add Bevy 0.18.1 and rosc 0.10.1 dependencies
- Set up basic Bevy app with 3D camera and ground plane

**Step 2: Implement `SynthTelemetry` resource and OSC receiver**
- `setup_osc_socket` startup system
- `receive_osc` system in `Update` schedule
- OSC message parsing with address matching
- Handle `/synth/meta` and `/synth/meta/seq` to detect protocol mismatches and drop old packets

**Step 3: Build FFT bar visualization**
- Spawn 128 cubes spread across X-axis
- `FftBar(index)` component on each
- `update_fft_bars` system reads `SynthTelemetry.fft`
- Optionally position bars using `fft_freqs` for log spacing

**Step 4: Add RMS light and note flash**
- Point light driven by RMS
- Emissive sphere that flashes on note events
- Color mapped from MIDI note number

**Step 5: Polish**
- Orbital camera (Bevy's `PanOrbitCamera` or custom)
- Bloom post-processing for glow effects
- Beat-synced background pulse using `beat_position`

### 2.7 Critical Review — Bevy Side

**What's good:**
- Clean ECS architecture — all state in `SynthTelemetry` resource, systems are pure functions
- Non-blocking UDP — `recv()` never stalls the render loop
- Drains all pending packets per frame — handles bursts without falling behind
- Decoupled from synth — works with any OSC sender following the address map
- Sequence handling (`/synth/meta/seq`) prevents out-of-order jitter

**Risks and mitigations:**
- **Risk:** Bevy `Update` runs at display refresh rate (60–144 Hz). OSC arrives at ~30 Hz.
  **Mitigation:** This is fine — visual systems interpolate/smooth. Some frames just reuse
  the previous telemetry data.
- **Risk:** UDP packets can arrive out of order or be dropped.
  **Mitigation:** Use `/synth/meta/seq` to drop old packets. For visualization, dropped packets
  just mean a brief stutter — acceptable.
- **Risk:** No synth running = no data = static scene.
  **Mitigation:** Default `SynthTelemetry` values are all zero. The scene renders but
  stays still. Could add a "waiting for signal" indicator.
- **Risk:** Bevy 0.18.x is the current stable (0.18.1 as of March 4, 2026), but Bevy's API churn is significant.
  **Mitigation:** The visualizer is a simple project. Upgrading Bevy versions is manageable
  since we only use basic Transform, PointLight, and Material systems.
- **Risk:** Protocol changes break the visualizer.
  **Mitigation:** Validate `protocol_version` from `/synth/meta` and show a warning overlay.

---

## Execution Order

### Phase 1: Synth OSC sender (this project)
1. Create `crates/synth_osc/` skeleton with config and address constants
2. Add second event ring buffer to `SynthEngine` (guarded by feature/option)
3. Implement the sender thread (event-rate loop + state-rate loop, `/synth/meta/seq`)
4. Expose master spectrum `VisualizationBuffer` for FFT data access
5. Wire into `pertylizer` main app with `--osc` CLI flag
6. Test with `oscdump` or a simple Python OSC listener

### Phase 2: Bevy visualizer (new project)
1. Scaffold Bevy project with camera and basic scene
2. Implement `SynthTelemetry` resource and OSC receiver system
3. Build FFT bar visualization
4. Add RMS-driven lighting and note flash effects
5. Add bloom, orbital camera, beat-sync pulse
6. Document OSC protocol in shared README

### Phase 3: Polish and extend
- Add more OSC addresses (per-instrument meters, envelope stages)
- Add configurable FFT bin count (64/128/256)
- Bevy: particle systems for note events
- Bevy: camera auto-movement synced to tempo
- Settings GUI toggle for OSC in Pertylizer

### Phase 4: Future ideas (optional)

**Additional effect concepts**

| Effect | Visual idea | Driven by | Technical approach |
|---|---|---|---|
| Spectral Origami | Folded planes that open with harmonics | FFT, centroid | Mesh folding + shader |
| Pulse Terrain | Landscape that breathes with bass | Low FFT, RMS | Heightmap displacement |
| Magnet Constellations | Stars cluster around notes | Note on/off, velocity | Instanced points + gravity |
| Chromatic Vortex | Spiral that accelerates on builds | Tempo, spectral flux | Spiral mesh + time warp |
| Glass Shatter | Transients break a glass shell | Peak, transient detect | Voronoi shards + impulse |
| Delay Weave | Delay lines render as braided threads | Echo/decay | Trail buffers + spline |
| Neon Calligraphy | Notes draw neon glyph strokes | Note on/off, pitch | SDF strokes + bloom |
| Aurora Curtains | Wide bands sweep with chords | Chord density | Ribbon mesh + gradient |
| Particle Resonator | Particles lock to modal frequencies | Modes, low FFT | Particle constraints |
| Fractal Pulse | Recursive shapes synced to beat | Tempo, RMS | Fractal instancing |

**Additional future features**
- Per-instrument OSC streams and per-track visual layers
- OSC-driven scene graph: spawn/despawn entities from synth events
- Multi-render targets: offscreen passes for bloom/blur and FFT-driven distortion
- Visual “macros” that can be automated like synth macros
- Parameter automation lanes for visuals in sync with transport
- Visual preset morphing and crossfade timeline
- Remote control API (WebSocket or MIDI) in addition to OSC
- Recording/export: render video to file for performances
- GPU FFT for higher-resolution spectra on the visualizer side

---

## Bandwidth Estimate

At 30 Hz update rate:

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
| **Total**        |             |        | **~22 KB/s** |

Well within localhost UDP capacity. No congestion risk.
