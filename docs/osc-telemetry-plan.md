# OSC Telemetry & Bevy Visualizer — Implementation Plan

## Overview

Add Open Sound Control (OSC) telemetry output to Pertylizer and build a separate Bevy 3D visualizer
that receives the data. The two projects communicate exclusively via UDP on `127.0.0.1:9000`.

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
   at ~30 Hz. This is exactly how the GUI already works. Zero changes to the audio thread.

2. **Discrete events** (note on/off, voice count, CPU) — we add a second optional
   `ringbuf::HeapRb<EngineEvent>` consumer in the engine. The audio thread pushes to both
   the GUI ring buffer and the OSC ring buffer (if enabled). The OSC sender thread drains
   this buffer each tick.

This approach keeps the audio thread allocation-free and lock-free. The only change in the
audio thread is one additional `try_push()` call per event — the same pattern already used
for the GUI event buffer.

### 1.2 OSC Address Map

| Address                     | Type          | Rate   | Source                    |
|-----------------------------|---------------|--------|---------------------------|
| `/synth/audio/rms`          | `[f, f]`      | ~30 Hz | `MeterState` atomics      |
| `/synth/audio/peak`         | `[f, f]`      | ~30 Hz | `MeterState` atomics      |
| `/synth/audio/fft`          | `[f * 128]`   | ~30 Hz | `VisualizationBuffer`     |
| `/synth/event/note_on`      | `[i, i, i]`   | Event  | `EngineEvent` ring buffer |
| `/synth/event/note_off`     | `[i, i]`      | Event  | `EngineEvent` ring buffer |
| `/synth/transport/state`    | `[i, f, f]`   | ~30 Hz | `SharedTransportState`    |
| `/synth/engine/voice_count` | `[i]`         | Event  | `EngineEvent` ring buffer |
| `/synth/engine/cpu`         | `[f]`         | ~1 Hz  | `EngineEvent` ring buffer |

**Argument details:**
- `/synth/audio/rms` — `(left: f32, right: f32)` linear amplitude 0.0–1.0+
- `/synth/audio/peak` — `(left: f32, right: f32)` linear amplitude
- `/synth/audio/fft` — 128 floats, normalized 0.0–1.0 (downsampled from 1025 FFT bins)
- `/synth/event/note_on` — `(midi_note: i32, velocity: i32, channel: i32)`
- `/synth/event/note_off` — `(midi_note: i32, channel: i32)`
- `/synth/transport/state` — `(playing: i32, tempo_bpm: f32, beat_position: f32)`
- `/synth/engine/voice_count` — `(count: i32)`
- `/synth/engine/cpu` — `(percent: f32)` 0.0–100.0

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
rosc = "0.10"                   # OSC encoding/decoding
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
    pub send_fft: bool,              // default true (can disable to save bandwidth)
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
- Spawn a thread that loops at `update_rate_hz`
- Each tick: read `MeterState` atomics → encode `/synth/audio/rms` and `/synth/audio/peak`
- Each tick: try read `VisualizationBuffer` snapshot → downsample 1025→128 bins → encode `/synth/audio/fft`
- Each tick: drain event ring buffer → encode note/voice/cpu messages
- Each tick: read `SharedTransportState` → encode `/synth/transport/state`
- Send all as an OSC bundle with timestamp via `UdpSocket`

**Step 3: Add a second event ring buffer in `SynthEngine`**
- Add an `Option<ringbuf::HeapProd<EngineEvent>>` field to `SynthEngine`
- When OSC is enabled, the engine clones events to this second producer
- The consumer half is passed to `OscTelemetry::start()`
- Guard with `if let Some(ref mut prod) = self.osc_event_producer`

**Step 4: Wire into the main application**
- In `pertylizer` crate, create `OscTelemetry` after engine starts
- Pass shared state references
- Add enable/disable toggle in settings (or just command-line flag initially)
- Stop on application exit

**Step 5: FFT data access**
- The master `SpectrumAnalyzer` already writes to a `VisualizationBuffer`
- Expose this buffer via `SynthEngine` so the OSC sender can read snapshots
- Downsample from 1025 complex bins to 128 bands (logarithmic grouping for perceptual accuracy)

### 1.7 Critical Review — Synth Side

**What's good:**
- Zero allocation in audio thread — we only add one `try_push()` call
- Reuses all existing data infrastructure (atomics, VisualizationBuffer)
- Clean separation — `synth_osc` depends on `synth_engine` but not `pertylizer`
- Graceful degradation — if UDP send fails, we just drop the packet (fire-and-forget)

**Risks and mitigations:**
- **Risk:** VisualizationBuffer uses `parking_lot::Mutex` internally — the OSC thread could
  contend with the GUI thread. **Mitigation:** Both use `try_lock()`, so neither blocks.
  Worst case: one frame gets a stale snapshot.
- **Risk:** FFT downsampling from 1025→128 bins could lose important frequency detail.
  **Mitigation:** Use logarithmic bin grouping (perceptual frequency scale), which matches
  how we hear. High frequencies have many bins averaged together, low frequencies keep detail.
- **Risk:** UDP packet size for FFT data: 128 floats × 4 bytes + OSC overhead ≈ 560 bytes.
  Well within the 1500-byte MTU. No fragmentation.
- **Risk:** Second event ring buffer doubles memory for events.
  **Mitigation:** Events are small (< 64 bytes each), buffer is 256 entries = ~16 KB. Negligible.

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
bevy = "0.16"
rosc = "0.10"
```

### 2.3 Core Resource

```rust
#[derive(Resource, Default)]
pub struct SynthTelemetry {
    /// RMS levels (left, right), linear amplitude.
    pub rms: [f32; 2],
    /// Peak levels (left, right), linear amplitude.
    pub peak: [f32; 2],
    /// FFT magnitude bands (128 bins), normalized 0.0–1.0.
    pub fft: [f32; 128],
    /// Most recent note-on event (MIDI note, velocity, channel).
    pub last_note_on: Option<(u8, u8, u8)>,
    /// Active voice count.
    pub voice_count: u32,
    /// Transport state.
    pub playing: bool,
    pub tempo: f32,
    pub beat_position: f32,
    /// CPU usage 0–100.
    pub cpu: f32,
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
    loop {
        match socket.socket.recv(&mut socket.buf) {
            Ok(size) => {
                if let Ok((_, packet)) = rosc::decoder::decode_udp(&socket.buf[..size]) {
                    handle_packet(&packet, &mut telemetry);
                }
            }
            Err(_) => break, // WouldBlock = no more data
        }
    }
    // Age note events
    telemetry.note_age_frames += 1;
}

fn handle_packet(packet: &OscPacket, telemetry: &mut SynthTelemetry) {
    match packet {
        OscPacket::Message(msg) => handle_message(msg, telemetry),
        OscPacket::Bundle(bundle) => {
            for p in &bundle.content {
                handle_packet(p, telemetry);
            }
        }
    }
}

fn handle_message(msg: &OscMessage, telemetry: &mut SynthTelemetry) {
    match msg.addr.as_str() {
        "/synth/audio/rms" => { /* extract f32, f32 → telemetry.rms */ }
        "/synth/audio/peak" => { /* extract f32, f32 → telemetry.peak */ }
        "/synth/audio/fft" => { /* extract 128 floats → telemetry.fft */ }
        "/synth/event/note_on" => { /* extract i32, i32, i32 → telemetry.last_note_on */ }
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
    if let Some((note, velocity, _)) = telemetry.last_note_on {
        if telemetry.note_age_frames < 2 {
            // Map MIDI note to hue (0–127 → 0°–360°)
            let hue = (note as f32 / 127.0) * 360.0;
            let brightness = velocity as f32 / 127.0;
            // Update material emissive color
        }
    }
}
```

### 2.6 Implementation Steps

**Step 1: `cargo init pertylizer-visualizer`**
- Add Bevy 0.16 and rosc dependencies
- Set up basic Bevy app with 3D camera and ground plane

**Step 2: Implement `SynthTelemetry` resource and OSC receiver**
- `setup_osc_socket` startup system
- `receive_osc` system in `Update` schedule
- OSC message parsing with address matching

**Step 3: Build FFT bar visualization**
- Spawn 128 cubes spread across X-axis
- `FftBar(index)` component on each
- `update_fft_bars` system reads `SynthTelemetry.fft`

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

**Risks and mitigations:**
- **Risk:** Bevy `Update` runs at display refresh rate (60–144 Hz). OSC arrives at ~30 Hz.
  **Mitigation:** This is fine — visual systems interpolate/smooth. Some frames just reuse
  the previous telemetry data.
- **Risk:** UDP packets can arrive out of order or be dropped.
  **Mitigation:** OSC bundles with timestamps allow the receiver to detect ordering. For
  visualization, dropped packets just mean a brief stutter — acceptable.
- **Risk:** No synth running = no data = static scene.
  **Mitigation:** Default `SynthTelemetry` values are all zero. The scene renders but
  stays still. Could add a "waiting for signal" indicator.
- **Risk:** Bevy 0.16 is the latest stable as of early 2026, but Bevy's API churn is significant.
  **Mitigation:** The visualizer is a simple project. Upgrading Bevy versions is manageable
  since we only use basic Transform, PointLight, and Material systems.

---

## Execution Order

### Phase 1: Synth OSC sender (this project)
1. Create `crates/synth_osc/` skeleton with config and address constants
2. Add second event ring buffer to `SynthEngine` (guarded by feature/option)
3. Implement the sender thread (poll atomics + drain events → encode → UDP send)
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

---

## Bandwidth Estimate

At 30 Hz update rate:

| Message          | Size (bytes) | Rate   | Bandwidth   |
|------------------|-------------|--------|-------------|
| RMS (2 floats)   | ~40         | 30 Hz  | 1.2 KB/s    |
| Peak (2 floats)  | ~40         | 30 Hz  | 1.2 KB/s    |
| FFT (128 floats) | ~560        | 30 Hz  | 16.8 KB/s   |
| Transport        | ~48         | 30 Hz  | 1.4 KB/s    |
| Note events      | ~40 each    | ~10/s  | 0.4 KB/s    |
| Voice count      | ~32         | ~5/s   | 0.16 KB/s   |
| CPU usage        | ~32         | 1 Hz   | 0.03 KB/s   |
| **Total**        |             |        | **~21 KB/s** |

Well within localhost UDP capacity. No congestion risk.
