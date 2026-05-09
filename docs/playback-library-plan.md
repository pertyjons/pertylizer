# Playback Library Plan (`synth_player`)

A standalone Rust crate for third-party programs (games, apps) to load and play back
pertylizer projects/songs with simultaneous sound effect playback.

## Goals

- Simple, high-level API for loading projects and controlling playback
- Simultaneous one-shot sound effects (layered on top of music)
- Low-latency, real-time safe audio mixing
- No GUI dependency — headless operation
- Cross-platform (Linux, macOS, Windows[playback-library-plan.md](playback-library-plan.md), WASM)
- Minimal public API surface

## Non-Goals

- Editing songs or patches at runtime
- Full MCP/OSC control
- Recording or MIDI input
- AWE (room simulation) — can be added later if needed

---

## Crate: `synth_player`

New crate at `crates/synth_player/`. Depends on:

| Dependency       | Purpose                                      |
|------------------|----------------------------------------------|
| `synth_engine`   | Audio engine, instruments, voice management   |
| `synth_sequencer`| Song, patterns, playback                      |
| `synth_core`     | Shared types (Gain, Hertz, etc.)              |
| `synth_modules`  | DSP modules for instrument playback           |
| `synth_dsp`      | Low-level DSP primitives                      |
| `cpal`           | Audio output backend                          |
| `serde_json`     | Project file loading                          |

Does **not** depend on `pertylizer` (GUI crate), `synth_awe`, `synth_mcp`, or `synth_osc`.

---

## Public API Design

```rust
/// Main entry point — manages audio output and mixing.
pub struct Player {
    // Owns cpal stream, engine handle, SFX mixer
}

/// Handle to a loaded song, ready for playback.
pub struct SongHandle { /* opaque */ }

/// Handle to a loaded sound effect.
pub struct SfxHandle { /* opaque */ }

/// Playback configuration.
pub struct PlayerConfig {
    pub sample_rate: Option<u32>,       // None = device default
    pub buffer_size: Option<u32>,       // None = device default
    pub max_sfx_voices: u16,            // default: 32
    pub master_volume: f32,             // 0.0–1.0, default: 0.8
}

/// Current playback state.
pub enum PlaybackState {
    Stopped,
    Playing,
    Paused,
}

/// Events emitted by the player.
pub enum PlayerEvent {
    SongFinished,
    SongLooped,
    Beat(u32),
    Bar(u32),
    SfxFinished(SfxHandle),
}
```

### Core API

```rust
impl Player {
    /// Create a new player with default audio output.
    pub fn new(config: PlayerConfig) -> Result<Self, PlayerError>;

    // --- Song Playback ---

    /// Load a project file (.json or .pertylizer bundle).
    pub fn load_song(&mut self, path: impl AsRef<Path>) -> Result<SongHandle, PlayerError>;

    /// Start playing the loaded song from the current position.
    pub fn play(&mut self, song: &SongHandle);

    /// Pause playback (resume with play()).
    pub fn pause(&mut self);

    /// Stop playback and rewind to start.
    pub fn stop(&mut self);

    /// Seek to a specific position (in beats).
    pub fn seek(&mut self, beat: f64);

    /// Set whether the song loops.
    pub fn set_looping(&mut self, looping: bool);

    /// Set song volume (0.0–1.0).
    pub fn set_song_volume(&mut self, volume: f32);

    /// Current playback state.
    pub fn state(&self) -> PlaybackState;

    /// Current playback position in beats.
    pub fn position_beats(&self) -> f64;

    /// Current tempo in BPM.
    pub fn tempo(&self) -> f64;

    /// Set playback tempo (overrides song tempo).
    pub fn set_tempo(&mut self, bpm: f64);

    // --- Sound Effects ---

    /// Load a sound effect from a WAV/OGG file.
    pub fn load_sfx(&mut self, path: impl AsRef<Path>) -> Result<SfxHandle, PlayerError>;

    /// Play a sound effect (fire-and-forget, mixed on top of music).
    pub fn play_sfx(&self, sfx: &SfxHandle) -> SfxInstanceId;

    /// Play a sound effect with parameters.
    pub fn play_sfx_with(&self, sfx: &SfxHandle, params: SfxParams) -> SfxInstanceId;

    /// Stop a specific SFX instance.
    pub fn stop_sfx(&self, instance: SfxInstanceId);

    /// Stop all playing sound effects.
    pub fn stop_all_sfx(&self);

    /// Set global SFX volume (0.0–1.0).
    pub fn set_sfx_volume(&mut self, volume: f32);

    // --- Global ---

    /// Set master volume (0.0–1.0).
    pub fn set_master_volume(&mut self, volume: f32);

    /// Poll for events (non-blocking).
    pub fn poll_events(&mut self) -> Vec<PlayerEvent>;
}

/// Per-instance SFX parameters.
pub struct SfxParams {
    pub volume: f32,        // 0.0–1.0, default: 1.0
    pub pan: f32,           // -1.0 (left) to 1.0 (right), default: 0.0
    pub pitch: f32,         // Playback rate multiplier, default: 1.0
    pub looping: bool,      // default: false
}
```

---

## Architecture

```
┌─────────────────────────────────────────────────┐
│                    Player                        │
│                                                  │
│  ┌──────────────┐    ┌──────────────────────┐   │
│  │  SynthEngine  │    │    SFX Mixer          │   │
│  │  (song        │    │  (sample playback,    │   │
│  │   playback)   │    │   voice pool,         │   │
│  │              │    │   pitch/pan/vol)       │   │
│  └──────┬───────┘    └──────────┬────────────┘   │
│         │                       │                │
│         └───────────┬───────────┘                │
│                     ▼                            │
│              Master Mixer                        │
│         (song + sfx → stereo out)                │
│                     │                            │
│                     ▼                            │
│              cpal audio stream                   │
└─────────────────────────────────────────────────┘
```

### Song Playback Path

Reuses existing infrastructure:

1. `ProjectFile::load()` → deserializes song, instruments, patches
2. Reconstruct `SynthEngine` + instruments from `InstrumentState` data
3. Set `Song` via `EngineCommand::SetSong`
4. `EngineCommand::Play/Stop/Pause/Seek` for transport control
5. `SynthEngine::process()` renders audio into buffer

The key challenge is reconstructing instruments from `InstrumentState`/`Patch` data
without the GUI session logic. This requires extracting the reconstruction code from
`pertylizer::session` into a shared utility, or duplicating the essential parts in
`synth_player`.

### SFX Mixer

New component, independent of `SynthEngine`:

- **Voice pool**: Pre-allocated array of `SfxVoice` structs (default 32)
- **Sample storage**: Loaded PCM data stored as `Arc<Vec<f32>>` (shared across instances)
- **Per-voice state**: playback position, volume, pan, pitch, looping flag
- **Lock-free trigger**: `play_sfx()` writes to a ring buffer, audio thread picks up
- **Real-time safe**: No allocations in the audio callback

```rust
struct SfxVoice {
    sample: Option<Arc<SfxSample>>,
    position: f64,          // fractional sample position
    volume: f32,
    pan: f32,
    pitch: f32,
    looping: bool,
    active: bool,
}

struct SfxSample {
    data: Vec<f32>,         // interleaved stereo PCM
    sample_rate: u32,
    channels: u16,
}
```

### Master Mixer

In the audio callback:

```rust
fn audio_callback(output: &mut [f32], context: &AudioCallbackContext) {
    // 1. Let SynthEngine render song into output buffer
    engine.process(output, context);

    // 2. Mix SFX on top (additive)
    sfx_mixer.process(&mut sfx_buffer, context);
    for i in 0..output.len() {
        output[i] += sfx_buffer[i];
    }

    // 3. Apply master volume + clipping
    for sample in output.iter_mut() {
        *sample = (*sample * master_volume).clamp(-1.0, 1.0);
    }
}
```

---

## Implementation Steps

### Phase 1: Minimal Song Playback

1. Create `crates/synth_player/` with `Cargo.toml`
2. Extract instrument reconstruction from `pertylizer::session` into reusable code
   (or into `synth_engine` as a `from_patch()` builder)
3. Implement `Player::new()` — cpal stream setup
4. Implement `load_song()` — project loading + engine setup
5. Implement `play/pause/stop/seek` — transport control via `EngineCommand`
6. Basic integration test: load a project, render N seconds to buffer

### Phase 2: Sound Effects

7. Implement `SfxSample` loading (WAV via `hound`, OGG via `lewton`)
8. Implement `SfxMixer` with voice pool and ring buffer triggers
9. Integrate SFX mixing into audio callback
10. Implement `SfxParams` (volume, pan, pitch, looping)
11. Test: play song + multiple simultaneous SFX

### Phase 3: Polish & Game Integration

12. `PlayerEvent` system (song finished, beat/bar callbacks)
13. Crossfade support for song transitions
14. Resource management (unload songs/sfx, memory limits)
15. Error handling and recovery (device lost, underruns)
16. C FFI wrapper (`synth_player_ffi`) for non-Rust consumers
17. Documentation and examples

### Phase 4: Optional Enhancements

18. Streaming playback (render song in chunks, don't load all samples upfront)
19. SFX spatial audio (3D positioning, distance attenuation)
20. SFX groups with shared volume control (music, sfx, voice, ambient)
21. WASM support (WebAudio backend)
22. Bevy/Godot integration examples

---

## Key Design Decisions

### Why a separate crate?

- Game developers don't want GUI dependencies
- Minimal dependency footprint
- Clean, focused API without synthesizer editing complexity
- Can be published independently on crates.io

### Why not just expose `SynthEngine` directly?

- `SynthEngine` + `EngineCommand` is low-level and complex
- Reconstructing instruments from project files requires session logic
- SFX playback doesn't exist in the current engine
- Games need a simpler mental model: load → play → done

### SFX mixing approach

Two options considered:

1. **Dedicated SFX mixer** (chosen) — separate voice pool, mixed after engine output
   - Pro: simple, no interaction with synth engine complexity
   - Pro: SFX work even without a loaded song
   - Pro: independent volume controls

2. **SFX as engine instruments** — load samples as Sampler modules
   - Con: heavyweight for simple one-shot effects
   - Con: uses synth voice allocation (overkill)
   - Con: couples SFX lifetime to engine state

### Thread safety

- `Player` is `!Send` (owns cpal stream)
- `SfxHandle` is `Clone + Send + Sync` (just an ID)
- `SongHandle` is `Clone + Send + Sync` (just an ID)
- `play_sfx()` is lock-free (ring buffer to audio thread)

---

## Example Usage

```rust
use synth_player::{Player, PlayerConfig, SfxParams};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create player
    let mut player = Player::new(PlayerConfig::default())?;

    // Load and play a song
    let song = player.load_song("assets/music/battle_theme.json")?;
    player.play(&song);
    player.set_looping(true);

    // Load sound effects
    let sword_sfx = player.load_sfx("assets/sfx/sword_hit.wav")?;
    let explosion_sfx = player.load_sfx("assets/sfx/explosion.ogg")?;

    // In game loop:
    loop {
        // Trigger SFX based on game events
        if sword_hit {
            player.play_sfx(&sword_sfx);
        }
        if explosion {
            player.play_sfx_with(&explosion_sfx, SfxParams {
                volume: 0.8,
                pan: enemy_x_position,
                pitch: 0.9 + rand::random::<f32>() * 0.2,
                ..Default::default()
            });
        }

        // Check for music events
        for event in player.poll_events() {
            match event {
                PlayerEvent::Beat(n) => sync_visual_to_beat(n),
                PlayerEvent::SongFinished => load_next_song(),
                _ => {}
            }
        }

        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}
```

---

## File Size Estimate

| Component                  | Estimated Lines |
|----------------------------|-----------------|
| `lib.rs` (public API)      | ~150            |
| `player.rs` (implementation)| ~400           |
| `sfx_mixer.rs`             | ~250            |
| `sfx_sample.rs` (loading)  | ~150            |
| `project_loader.rs`        | ~200            |
| `error.rs`                 | ~50             |
| `events.rs`                | ~80             |
| Tests                      | ~300            |
| **Total**                  | **~1,580**      |
