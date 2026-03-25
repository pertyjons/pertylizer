# Sampling & Audio Recording — Implementation Plan

> **Date:** 2025-03-25
> **Status:** Draft
> **Scope:** New `Sample` view, sample library, WAV import/export, sampler module, live audio input, project format v2

---

## Table of Contents

1. [Overview](#1-overview)
2. [New Crate: `synth_sampler`](#2-new-crate-synth_sampler)
3. [Sample Library (shared data store)](#3-sample-library-shared-data-store)
4. [Audio Input (recording)](#4-audio-input-recording)
5. [Sample View (new GUI tab)](#5-sample-view-new-gui-tab)
6. [Sampler Module (Rack patch module)](#6-sampler-module-rack-patch-module)
7. [Project Format v2 (bundle format)](#7-project-format-v2-bundle-format)
8. [Engine Commands & Events](#8-engine-commands--events)
9. [MCP Integration (full remote control)](#9-mcp-integration-full-remote-control)
10. [Implementation Phases](#10-implementation-phases)
11. [Important Features You May Have Missed](#11-important-features-you-may-have-missed)
12. [Future Feature Ideas](#12-future-feature-ideas)

---

## 1. Overview

The goal is to add **sampling** as a first-class feature in Pertylizer. This includes:

- **Recording audio** from an external input (microphone, line-in)
- **Importing/exporting** WAV files
- **A dedicated Sample view** for managing, editing, and previewing samples
- **A Sampler patch module** usable in the Rack view to play samples as instruments
- **Live audio input** passthrough into the patch graph for real-time monitoring
- **Bundle project format** that embeds samples alongside instrument/song data

### Architecture Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                     Sample Library                          │
│  (Arc<RwLock<SampleLibrary>>  — shared between all threads) │
│  ┌──────────┐ ┌──────────┐ ┌──────────┐                     │
│  │ Sample 0 │ │ Sample 1 │ │ Sample 2 │  ...                │
│  │ Arc<[f32]│ │ Arc<[f32]│ │ Arc<[f32]│                     │
│  └──────────┘ └──────────┘ └──────────┘                     │
└───────┬────────────────┬────────────────┬───────────────────┘
        │                │                │
   ┌────▼────┐    ┌──────▼──────┐   ┌────▼──────────┐
   │ Sample  │    │   Sampler   │   │  Audio Input  │
   │  View   │    │   Module    │   │  (cpal input) │
   │  (GUI)  │    │  (in Rack)  │   │               │
   └─────────┘    └─────────────┘   └───────────────┘
```

---

## 2. New Crate: `synth_sampler`

A new crate for sample data types and DSP, keeping `synth_core` lean.

### 2.1 Core Types

```rust
// synth_sampler/src/types.rs

/// Unique identifier for a sample in the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SampleId(pub u64);

// ChannelCount is re-exported from synth_core::audio::types::ChannelCount (Mono, Stereo).
// Do NOT define a new one — use: `pub use synth_core::ChannelCount;`

/// Frame index within a sample buffer (absolute position).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[must_use]
pub struct FrameIndex(pub usize);

/// Fractional playback position within a sample (sub-frame precision).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct PlaybackPosition(pub f64);

/// Playback speed ratio (1.0 = original pitch, 2.0 = one octave up).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
#[must_use]
pub struct PlaybackSpeed(pub f64);

/// A sample's metadata (does NOT contain audio data).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleMeta {
    pub id: SampleId,
    pub name: String,
    pub sample_rate: SampleRate,
    pub channels: ChannelCount,
    pub frame_count: FrameCount,      // total frames (from synth_core)
    pub duration: Seconds,
    pub root_note: Option<MidiNote>,  // pitch mapping
    pub loop_region: Option<LoopRegion>,
    pub crop: Option<CropRegion>,     // audible region (serialized in bundle metadata.json)
    pub source: SampleSource,         // where it came from
}

/// How was this sample created?
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SampleSource {
    /// Recorded from audio input in this session.
    Recorded,
    /// Imported from a WAV file. Path is informational only (may not exist on other machines).
    Imported { original_path: Option<PathBuf> },
    /// Generated programmatically (future: text-to-speech, synthesis render).
    Generated,
}

/// Loop region within a sample.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LoopRegion {
    pub start: FrameIndex,
    pub end: FrameIndex,         // exclusive
    pub crossfade: FrameCount,   // frames of crossfade to avoid clicks
}

/// Crop region — the audible portion of the full sample buffer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CropRegion {
    pub start: FrameIndex,
    pub end: FrameIndex,         // exclusive
}
```

**Note on newtypes:** `FrameCount` is re-exported from `synth_core`. `FrameIndex`, `PlaybackPosition`,
and `PlaybackSpeed` are new newtypes defined in `synth_sampler` (or promoted to `synth_core` if reuse warrants it).
All frame-based fields use `FrameIndex` or `FrameCount` — never raw `usize`.

**IMPORTANT: `FrameCount` in `synth_core` currently lacks `Serialize, Deserialize`.** These derives
must be added before `FrameCount` can be used in `SampleMeta`, `LoopRegion`, etc. This is a small
change to `synth_core/src/types/audio.rs`: add `Serialize, Deserialize` to the derive and
`#[serde(transparent)]` so it serializes as a plain `usize` (not `{"0": 1234}`).

### 2.2 Sample Data

```rust
// synth_sampler/src/sample.rs

/// A loaded sample with audio data. Audio data is Arc-shared for
/// zero-copy access from multiple voices/modules on the audio thread.
pub struct Sample {
    pub meta: SampleMeta,
    /// Interleaved audio data (mono: [L,L,L,...], stereo: [L,R,L,R,...]).
    /// Arc allows zero-copy sharing across voices without allocation.
    pub data: Arc<[f32]>,
    // Note: crop is stored in meta.crop (single source of truth, serialized with metadata)
}
```

### 2.3 WAV I/O

```rust
// synth_sampler/src/wav.rs

/// Load a WAV file into a Sample. Resamples to target_rate if needed.
pub fn load_wav(path: &Path, target_rate: SampleRate) -> Result<Sample, SampleError>;

/// Save a Sample (or region thereof) to a WAV file.
pub fn save_wav(sample: &Sample, path: &Path, bit_depth: BitDepth) -> Result<(), SampleError>;
```

Uses the existing `hound` dependency. Must handle:

- 8/16/24-bit PCM and 32-bit float input
- Sample rate conversion (linear interpolation for MVP, sinc later)
- Mono/stereo detection

### 2.4 Sample Playback DSP

```rust
// synth_sampler/src/playback.rs

/// Playback state for one voice playing a sample.
pub struct SamplePlayer {
    sample: Arc<[f32]>,
    channels: ChannelCount,
    position: PlaybackPosition,  // fractional frame position (sub-sample precision)
    speed: PlaybackSpeed,        // playback speed ratio (1.0 = original pitch)
    crop: CropRegion,
    loop_region: Option<LoopRegion>,
    play_mode: PlayMode,
    state: PlaybackState,
}

pub enum PlaybackState {
    Playing,
    Looping,
    Finished,
}

impl SamplePlayer {
    /// Render `count` stereo frames into `output`. Returns true if still active.
    /// Uses cubic (Hermite) interpolation between frames for anti-aliased pitch shifting.
    /// Linear interpolation fallback available for lower CPU usage.
    pub fn render(&mut self, output: &mut [f32], count: FrameCount) -> bool;

    /// Set playback speed ratio for pitch shifting.
    /// speed = 2^((target_note - root_note) / 12.0)
    pub fn set_pitch(&mut self, target_note: MidiNote, root_note: MidiNote);
}
```

**Real-time safe**: no allocations, `Arc<[f32]>` is a shared pointer (clone is atomic increment only), position tracking
is just arithmetic.

---

## 3. Sample Library (shared data store)

The sample library is the central registry of all loaded samples, shared between the GUI and audio engine.

### 3.1 Structure

```rust
// synth_sampler/src/library.rs

pub struct SampleLibrary {
    samples: HashMap<SampleId, Sample>,
    next_id: SampleId,
}

impl SampleLibrary {
    pub fn add(&mut self, sample: Sample) -> SampleId;
    pub fn remove(&mut self, id: SampleId) -> Option<Sample>;
    pub fn get(&self, id: SampleId) -> Option<&Sample>;
    pub fn get_data(&self, id: SampleId) -> Option<Arc<[f32]>>;
    pub fn get_meta(&self, id: SampleId) -> Option<&SampleMeta>;
    pub fn list(&self) -> Vec<&SampleMeta>;
    pub fn update_meta(&mut self, id: SampleId, meta: SampleMeta);
    pub fn update_crop(&mut self, id: SampleId, crop: Option<CropRegion>);
    pub fn update_loop(&mut self, id: SampleId, region: Option<LoopRegion>);
}
```

### 3.2 Thread Sharing Strategy

```
GUI thread  ─── Arc<RwLock<SampleLibrary>> ─── write() for add/remove/edit
Audio thread ──────────────────────────────── try_read() for sample lookups (never blocks)
```

When a sample is loaded/recorded, the GUI thread writes to the library. The audio thread only needs the `Arc<[f32]>`
data pointer, which it obtains once and holds independently. Metadata changes (crop, loop) are communicated to the audio
thread via `EngineCommand`.

### 3.3 Sending Samples to the Audio Thread

Samples cannot be loaded on the audio thread. Instead:

1. GUI loads WAV → creates `Sample` with `Arc<[f32]>` data
2. GUI adds to `SampleLibrary`
3. GUI sends `EngineCommand::LoadSample { id, data: Arc<[f32]>, meta }` to audio thread
4. Audio thread stores the `Arc<[f32]>` in a local cache for instant playback
5. When sample is removed, send `EngineCommand::UnloadSample { id }`

This way the audio thread never reads the `RwLock` — it has its own copy of the data pointers.

---

## 4. Audio Input (recording)

### 4.1 Extend AudioBackend Trait

The `AudioBackend` trait already has `devices()` (returns all devices with `DeviceType`) and
`default_input_device()`. Input devices can be found by filtering `devices()` results on
`DeviceType::Input` or `DeviceType::Duplex` — **no new trait methods needed for device enumeration**.

The only addition needed is an input stream method:

```rust
// synth_core/src/audio/traits.rs — add to AudioBackend

/// Start an input stream that writes captured audio into the provided ring buffer producers.
/// Two producers are needed because ringbuf is SPSC: one for the engine (low-latency passthrough)
/// and one for the GUI (metering + recording). The cpal callback pushes each sample to both.
fn start_input(
    &mut self,
    device: Option<&str>,
    config: &StreamConfig,
    engine_producer: ringbuf::HeapProd<f32>,
    gui_producer: ringbuf::HeapProd<f32>,
) -> AudioResult<Box<dyn AudioStream>>;
```

Note: both `HeapProd<f32>` halves are moved into the cpal input callback closure.
The corresponding `HeapCons<f32>` halves stay with the engine and `AudioInputManager` respectively.

### 4.2 Audio Input Manager

```rust
// pertylizer/src/audio/input.rs

pub struct AudioInputManager {
    /// GUI-side ring buffer consumer (large buffer, ~65536 samples).
    /// Drained at ~60fps for metering and recording. Separate from the engine consumer.
    gui_consumer: ringbuf::HeapCons<f32>,
    /// Engine-side consumer to send to SynthEngine (small buffer, ~2048 samples).
    /// Created on start_monitoring(), sent via EngineCommand::SetAudioInputConsumer.
    engine_consumer: Option<ringbuf::HeapCons<f32>>,
    /// Current input stream (None when not monitoring)
    stream: Option<Box<dyn AudioStream>>,
    /// Recording state
    state: InputState,
    /// Accumulated recording buffer (pre-allocated with capacity)
    record_buffer: Vec<f32>,
    /// Peak level for metering (atomic, read from GUI)
    peak_level: Arc<AtomicU32>,  // f32 as bits
}

pub enum InputState {
    Idle,
    Monitoring,     // input is open but not recording
    Recording,      // actively capturing to record_buffer
}

impl AudioInputManager {
    pub fn start_monitoring(&mut self) -> AudioResult<()>;
    pub fn stop_monitoring(&mut self);
    pub fn start_recording(&mut self);
    pub fn stop_recording(&mut self) -> Option<Vec<f32>>;  // returns captured audio
    pub fn read_available(&mut self) -> &[f32]; // for live passthrough
    pub fn peak_level(&self) -> f32;
}
```

### 4.3 Live Input to Engine (passthrough)

For the "test in patch view" use case, the audio input needs to reach the Sampler module in real-time:

```
Microphone → cpal input callback → HeapProd<f32> ──→ HeapCons<f32> (in SynthEngine)
                                                           │
                                                     drain once per block
                                                           │
                                                           ▼
                                                   audio_input_buffer: [f32; BLOCK_SIZE * 2]
                                                           │
                                              ┌────────────┼────────────┐
                                              ▼            ▼            ▼
                                         Voice 0       Voice 1      Voice 2
                                      AudioInputModule (reads/copies from shared buffer)
```

**CRITICAL architecture note:** `PolyModule` is instantiated per-voice. If the instrument has 8 voices
and 3 keys are held, 3 separate `AudioInputModule` instances will call `process()`. They **cannot**
each hold a `HeapCons` — it is not `Clone`, not `Send`-safe without a Mutex, and the first voice
would consume all data leaving the others silent.

**Solution:** Two-stage design with dual ring buffers:

#### Stage 1: Dual Ring Buffers (SPSC constraint)

`ringbuf` is strictly SPSC (Single-Producer Single-Consumer). The engine and the GUI both need
to read from the mic input, but they **cannot share one consumer**. Solution: the cpal input
callback writes to **two separate ring buffers**:

```
                                    ┌─── engine_prod ──→ engine_cons (in SynthEngine)
Microphone → cpal callback ────────┤                      small buffer (~2048 samples)
                                    └─── gui_prod ────→ gui_cons (in AudioInputManager)
                                                          large buffer (~65536 samples)
```

- **`engine_cons`**: Small, low-latency buffer. Engine drains it once per block for live passthrough.
- **`gui_cons`**: Large buffer. GUI thread drains at ~60fps for metering and, when recording,
  appends data to the pre-allocated `record_buffer: Vec<f32>`. This completely separates the
  real-time engine from GUI-side memory allocations.

#### Stage 2: Engine distributes to voices

`SynthEngine::process()` drains `engine_cons` once per block into a pre-allocated
`audio_input_buffer: Vec<f32>` (sized to `block_size * 2` for stereo). This happens before
any voice processing.

`AudioInputModule` (the `PolyModule` in each voice) receives a read-only reference to this
shared buffer via the `ProcessContext`. All voices read (copy) the same audio data. This is
lock-free and lets multiple voices process live input in parallel (e.g., 3 keys → 3 different
filter/envelope paths on the same mic signal).

**Important voice behavior:** Since `PolyModule` instances only run in *active* voices, the
`AudioInputModule` will only output audio when a key is held (or a note is sustained). This is
consistent with how the existing `Noise` module works. The user must play a note to "open" the
voice and hear the mic through the patch. For an always-on mic bypass, use an `AudioEffect`
instead (future feature).

#### ProcessContext extension

The current `ProcessContext` has no lifetime parameter. Adding `audio_input: Option<&'a [f32]>`
requires changing it to `ProcessContext<'a>`, which propagates to the `PolyModule::process()`
and `AudioEffect::process()` trait signatures — a mechanical refactor touching all ~35 existing
modules, plus tests, benchmarks (`benches/audio_profile.rs`), mock modules, and anywhere
`ProcessContext` is constructed (primarily `SynthEngine`). Signature change only, no logic change.
This is the cleanest approach — do as a standalone commit at the start of Phase 5.

```rust
// synth_core/src/module_traits.rs — updated
#[derive(Debug, Clone, Copy)]
pub struct ProcessContext<'a> {
    pub sample_rate: SampleRate,
    pub samples: SampleCount,
    pub tempo: Bpm,
    pub is_playing: bool,
    pub position_beats: BeatPosition,
    /// Live audio input buffer for the current block (None if no input active).
    pub audio_input: Option<&'a [f32]>,
}

// PolyModule trait signature becomes:
fn process(&mut self, inputs: InputPorts<'_>, outputs: &mut HashMap<PortName, AudioBuffer>, context: &ProcessContext<'_>);

// New EngineCommand — sends the consumer half to the engine (once, at setup)
EngineCommand::SetAudioInputConsumer { consumer: ringbuf::HeapCons<f32> }
```

#### SampleId and the parameter system

The `get_param()` trait method returns `Option<f32>`, which loses precision for `SampleId(u64)`.
However, **no dedicated `AssignSample` command is needed**. The existing `SetModuleParameter`
command sends the full typed `Param` enum — not `f32`:

```rust
EngineCommand::SetModuleParameter {
instrument_id: Some(id),
module_id: smp_1,
param: Param::Sampler(SamplerParam::SampleSelect(SampleId(42))),
}
```

This works because `set_param()` receives the typed `Param`, and Patch serialization also uses
`Param` (via `get_params() -> Vec<Param>`). Only `get_param()` returns `f32` — it should return
`None` for `SampleSelect` (it's not a slider-compatible parameter). The GUI sample dropdown
reads via `get_params()` instead.

This means sample assignment follows the **standard module parameter flow** — no special-casing
needed in the engine command dispatch, patch save/load, or MCP bridge.

---

## 5. Sample View (new GUI tab)

### 5.1 Navigation

Add `Sample` variant to `AppView`:

```rust
pub enum AppView {
    Rack,
    AcousticWorld,
    Sequencer,
    Sample,        // NEW
}
```

Accessible via the top panel tabs, same pattern as AWE and Sequencer.

### 5.2 View Layout

```
┌──────────────────────────────────────────────────────────────┐
│ [Rack] [AWE] [Seq] [Sample]           ▶ ■ ● REC   BPM: 120 │  ← Top bar
├─────────────┬────────────────────────────────────────────────┤
│             │  ┌──── Waveform Display ─────────────────────┐ │
│  Sample     │  │  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓ │ │
│  List       │  │  [crop handles]   [loop markers]          │ │
│             │  │  ◄────── zoom scroll ────────►            │ │
│  ┌────────┐ │  └───────────────────────────────────────────┘ │
│  │ Vocal  │ │                                                │
│  │ 2.3s   │ │  ┌──── Properties ──────────────────────────┐  │
│  ├────────┤ │  │ Name: [Vocal Take 1    ]                 │  │
│  │ Kick   │ │  │ Root Note: [C4 ▾]  Duration: 2.3s       │  │
│  │ 0.4s   │ │  │ Sample Rate: 48000  Channels: Stereo    │  │
│  ├────────┤ │  │ Loop: [●] Start: 0.5s End: 2.1s         │  │
│  │ Pad    │ │  │ Crossfade: [12ms ▾]                      │  │
│  │ 5.1s   │ │  └──────────────────────────────────────────┘  │
│  ├────────┤ │                                                │
│  │ + Add  │ │  ┌──── Toolbar ─────────────────────────────┐  │
│  │ ● Rec  │ │  │ [▶Play] [■Stop] [Zoom+] [Zoom-] [FitAll]│  │
│  └────────┘ │  │ [Import WAV] [Export WAV] [Crop] [Delete]│  │
│             │  └──────────────────────────────────────────┘  │
├─────────────┴────────────────────────────────────────────────┤
│ Input: [Default Mic ▾]  Level: ▓▓▓▓░░░░  [● Monitor]       │  ← Bottom bar
└──────────────────────────────────────────────────────────────┘
```

### 5.3 Components

#### 5.3.1 Sample List (left panel)

- Lists all samples in the library by name and duration
- Click to select and display in waveform view
- "+" button to import WAV
- "● Rec" button to start recording from input
- Right-click context menu: Rename, Duplicate, Delete, Export WAV
- Drag-and-drop reordering
- Visual indicator showing which samples are used by instruments
- **Memory usage** — show total sample memory at the bottom of the list (e.g., "5 samples — 12.4 MB")

#### 5.3.2 Waveform Display (central panel)

- Uses the existing `waveform.rs` widget as a starting point, extended for:
    - **Zoom** — horizontal zoom with scroll wheel, vertical zoom with Ctrl+scroll
    - **Scroll** — pan through zoomed waveform with middle-click drag or scrollbar
    - **Crop handles** — draggable start/end markers that define the audible region
    - **Loop markers** — draggable loop start/end within the crop region, displayed as colored overlay
    - **Playback cursor** — moving cursor during preview playback
    - **Selection** — click+drag to select a region (for future cut/copy/paste)
- Renders waveform from `Arc<[f32]>` data with downsampled peak cache for performance
- Stereo: display L/R channels stacked or overlaid (toggle)

#### 5.3.3 Properties Panel

- **Name** — editable text field
- **Root note** — MIDI note selector (determines what pitch the sample plays at)
- **Loop toggle** — enable/disable looping
- **Loop start/end** — numeric input (or drag in waveform)
- **Crossfade** — crossfade length at loop boundary to prevent clicks
- **Sample info** — read-only: sample rate, channels, duration, file size

#### 5.3.4 Toolbar

- **Play/Stop** — preview the selected sample through the master output
- **Zoom controls** — zoom in/out, fit all
- **Import WAV** — file dialog to load `.wav` files
- **Export WAV** — save selected sample (with crop applied) as `.wav`
- **Crop** — apply crop destructively (trim the buffer)
- **Normalize** — normalize peak level to 0 dB
- **Reverse** — reverse the sample data
- **Auto-trim** — detect silence threshold and set crop markers to first/last audible frame (threshold configurable,
  default -40 dB)
- **Delete** — remove sample from library

#### 5.3.5 Input Monitor Bar (bottom)

- **Input device selector** — dropdown of available input devices
- **Level meter** — real-time input level display
- **Monitor toggle** — hear the input through speakers
- **Latency warning** — when monitoring is enabled and output latency >20ms, show a warning badge with the latency
  value (e.g., "⚠ 23ms"). Suggest reducing buffer size in settings if possible.
- **Record button** — start/stop recording to a new sample
- **Record timer** — elapsed time display during recording

### 5.4 Waveform Peak Cache

For large samples, drawing every sample is too slow. Pre-compute a peak cache:

```rust
/// Downsampled peak data for efficient waveform rendering.
pub struct WaveformPeaks {
    /// For each "column" of pixels at a given zoom level: (min, max) amplitude pair.
    peaks: Vec<(Amplitude, Amplitude)>,
    /// How many source frames each peak entry represents.
    frames_per_peak: FrameCount,
}
```

Generate multiple mip-map levels (e.g., 256, 1024, 4096 frames/peak) on import. GUI picks the level closest to the
current zoom.

---

## 6. Sampler Module (Rack patch module)

### 6.1 Module Type

Add a new `ModuleType::Sampler` variant. This is a **voice module** (implements `PolyModule`) that can be placed in the
Rack view's patch editor, just like an oscillator.

### 6.2 Parameters

**IMPORTANT:** `SamplerParam`, `PlayMode`, and `PlayDirection` must be defined in
**`synth_core::params::sampler`** (not in `synth_sampler`) because they are variants inside the
`Param` enum which lives in `synth_core`. Placing them in `synth_sampler` would create a circular
dependency (`synth_core` → `synth_sampler` → `synth_core`). The `synth_sampler` crate handles
only DSP and data (`Sample`, `SamplePlayer`, WAV I/O).

```rust
// synth_core/src/params/sampler.rs — NEW FILE

pub enum SamplerParam {
    /// Which sample to play (by SampleId).
    SampleSelect(SampleId),
    /// Pitch tracking — follow MIDI note or play at fixed pitch.
    PitchTracking(bool),
    /// Playback start offset (0.0 = beginning of crop, 1.0 = end).
    StartOffset(NormalizedValue),
    /// Volume/gain.
    Level(Gain),
    /// Loop on/off (uses sample's loop region if defined).
    LoopEnabled(bool),
    /// Playback mode — how the sample responds to NoteOn/NoteOff.
    PlayMode(PlayMode),       // OneShot, Sustain, Loop
    /// Playback direction.
    Direction(PlayDirection),  // Forward, Reverse, PingPong
    /// Velocity sensitivity (how much velocity affects volume).
    VelocitySensitivity(NormalizedValue),
    /// Fine-tune in cents (-100 to +100), same as oscillator modules.
    FineTune(Cents),
}

// NOTE: PlayMode, PlayDirection, and SamplerParam must all derive Copy + Serialize + Deserialize
// because the parent `Param` enum is `#[derive(Copy, Serialize, Deserialize)]`.
// All inner types (SampleId, NormalizedValue, Gain, Cents, bool) are already Copy.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayMode {
    /// Play the full sample regardless of NoteOff (drums, percussion, one-hit SFX).
    OneShot,
    /// Play until NoteOff, then stop (or enter release). Default for melodic samples.
    Sustain,
    /// Play to loop end, loop back to loop start until NoteOff, then release.
    Loop,
}

/// Playback direction. New type — does NOT exist in the current codebase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayDirection {
    Forward,
    Reverse,
    PingPong,
}
```

### 6.3 Signal Flow

```
MIDI NoteOn → Sampler Module → [audio out] → Filter → Amplifier → ...
                    ↑
              Arc<[f32]> from SampleLibrary
```

The Sampler module:

1. On `NoteOn`: creates a `SamplePlayer` for the triggered note
2. During `process()`: renders sample audio using pitch-shifted playback
3. On `NoteOff`: if not looping, enters release (or stops immediately)
4. Outputs audio through its `"out"` port, connectable like any oscillator

### 6.4 Multi-sample Support (stretch goal)

For more realistic instruments, support multiple samples mapped across the keyboard:

```rust
pub struct SampleMapping {
    pub sample_id: SampleId,
    pub key_low: MidiNote,
    pub key_high: MidiNote,
    pub velocity_low: Velocity,
    pub velocity_high: Velocity,
    pub root_note: MidiNote,
}
```

This allows creating instruments like a piano where different samples cover different ranges.

### 6.5 Module UI in Rack

The Sampler module panel in the Rack view shows:

- Sample name (dropdown to select from library)
- Mini waveform preview
- Start offset knob
- Level knob
- Fine-tune knob (Cents)
- Play mode selector (OneShot / Sustain / Loop)
- Direction selector (Forward / Reverse / PingPong)
- "Edit in Sample View" button (switches to Sample tab with this sample selected)

---

## 7. Project Format v2 (bundle format)

### 7.1 Problem

Current format: single `.json` file. Samples are binary data (potentially megabytes) that doesn't belong in JSON.

### 7.2 Solution: ZIP-based Bundle

Use a `.pertproj` extension (ZIP archive internally):

```
project.pertproj (ZIP)
├── project.json          ← existing ProjectFile structure (text)
├── samples/
│   ├── 0001_vocal.wav    ← sample audio data as WAV
│   ├── 0002_kick.wav
│   └── 0003_pad.wav
└── metadata.json         ← sample library metadata (SampleMeta for each)
```

### 7.3 Format Details

**`project.json`** — same structure as today's `ProjectFile`, with one addition:

```rust
pub struct ProjectFile {
    pub file_type: String,        // "project"
    pub version: String,          // "2.0"
    pub instruments: Vec<InstrumentState>,
    pub active_instrument_id: u64,
    pub author: Option<Author>,
    pub song: Song,
    pub global: GlobalProjectState,
    // NEW: sample references used by instruments
    pub sample_refs: Vec<SampleRef>,
}

pub struct SampleRef {
    pub id: SampleId,
    /// Path within the bundle (e.g., "samples/0001_vocal.wav").
    pub bundle_path: String,
}
```

**`metadata.json`** — array of `SampleMeta` for each sample in the bundle, including loop points, crop regions, root
notes, etc.

### 7.4 Backward Compatibility

- v1 files (plain JSON with no samples) continue to load normally
- **IMPORTANT:** The current `load_file()` in `project.rs` uses `fs::read_to_string()` which will
  fail with a UTF-8 error on ZIP files. Must change to `fs::read()` (raw bytes) first, then:
    1. Inspect first 2 bytes: `PK` (0x50 0x4B) → ZIP bundle → unpack and parse
    2. Otherwise → convert bytes to string (`String::from_utf8`) → parse as JSON (legacy format)
- Save always uses bundle format when samples exist; falls back to plain JSON if no samples

### 7.5 Implementation

Use the `zip` crate (add to workspace dependencies):

```toml
[dependencies]
zip = "2"
```

```rust
pub fn save_bundle(project: &ProjectFile, samples: &SampleLibrary, path: &Path) -> Result<()>;
pub fn load_bundle(path: &Path) -> Result<(ProjectFile, Vec<Sample>)>;
```

### 7.6 Patch Files with Samples

Single-instrument patches that use samples also need bundling:

```
instrument.pertpatch (ZIP)
├── patch.json           ← existing Patch structure
├── samples/
│   └── 0001_bass.wav
└── metadata.json
```

---

## 8. Engine Commands & Events

### 8.1 New Commands

```rust
// Add to EngineCommand enum

/// Load a sample's audio data into the engine's sample cache.
LoadSample {
id: SampleId,
data: Arc<[f32] >,
channels: ChannelCount,
sample_rate: SampleRate,
frame_count: FrameCount,
crop: Option<CropRegion>,
loop_region: Option<LoopRegion>,
},
/// Remove a sample from the engine's cache.
UnloadSample {
id: SampleId,
},
/// Update a sample's loop region (from GUI edit).
UpdateSampleLoop {
id: SampleId,
loop_region: Option<LoopRegion>,
},
/// Update a sample's crop region (from GUI edit).
UpdateSampleCrop {
id: SampleId,
crop: Option<CropRegion>,
},

// Note: sample assignment uses the existing SetModuleParameter command with
// Param::Sampler(SamplerParam::SampleSelect(SampleId)) — no dedicated command needed.

/// Send the ring buffer consumer to the engine for live audio input.
/// The engine drains this once per block into a shared audio_input_buffer
/// that all AudioInputModule voice instances can read from.
SetAudioInputConsumer {
consumer: ringbuf::HeapCons<f32>,
},
/// Preview a sample through the master output (from Sample view).
/// Routed through a dedicated `preview_player` inside SynthEngine that
/// bypasses the instrument graph entirely — mixed directly into the
/// master mix buffer before master effects. This prevents the preview
/// from stealing voice resources from active instruments.
///
/// NOTE: This is DIFFERENT from the existing `audio::preview::render_note_preview()`
/// which renders a note offline to a WAV buffer for MCP. This command triggers
/// real-time playback of a raw sample for auditioning in the Sample view.
PreviewSample {
id: SampleId,
},
/// Stop sample preview.
StopPreview,
```

### 8.2 New Events

```rust
// Add to EngineEvent enum

/// Sample preview finished playing.
PreviewFinished {
id: SampleId,
},
```

**Note on input metering:** Audio input levels for the Sample view are computed on the **GUI thread**
by draining the `gui_consumer` ring buffer — NOT via `EngineEvent`. The `AudioInputManager` computes
peak/RMS from the drained samples and exposes it via `AudioInputManager::peak_level()`. This avoids
flooding the event ring buffer with per-block meter updates and keeps metering independent of the engine.

### 8.3 EngineCommand Debug implementation

`EngineCommand` has a **manual `impl Debug`** (because `Box<dyn PolyModule>` is not `Debug`).
All new command variants (`LoadSample`, `UnloadSample`, `UpdateSampleLoop`, `UpdateSampleCrop`,
`SetAudioInputConsumer`, `PreviewSample`, `StopPreview`) must be added to this manual `Debug`
impl, otherwise the code will not compile.

---

## 9. MCP Integration (full remote control)

Following the same pattern as the AWE MCP integration, the sampling system gets a complete set of MCP tools so that AI
agents (and other MCP clients) can manage samples, control recording, assign samples to instruments, and edit sample
properties — all remotely.

### 9.1 New MCP Tools

#### Sample Library Management

| Tool               | Description                                                                                                                                                                                |
|--------------------|--------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `list_samples`     | List all samples in the library with metadata (id, name, duration, channels, sample rate, root note, loop settings, crop region, source type). Supports optional filter by name substring. |
| `get_sample_info`  | Get detailed metadata for a single sample by id, including full waveform statistics (peak level, RMS, DC offset), memory usage, and which instruments reference it.                        |
| `import_sample`    | Import a WAV file from a given path into the sample library. Returns the new `SampleId` and metadata. Accepts optional `name` and `root_note` overrides.                                   |
| `delete_sample`    | Remove a sample from the library by id. Returns error if sample is in use by an instrument (with `force: bool` to override).                                                               |
| `rename_sample`    | Rename a sample by id.                                                                                                                                                                     |
| `duplicate_sample` | Create a copy of a sample (with new id) — useful for creating variations.                                                                                                                  |
| `export_sample`    | Export a sample (with crop applied) to a WAV file at a given path. Accepts `bit_depth` parameter (16/24/32f).                                                                              |

#### Sample Editing

| Tool                   | Description                                                                                                                    |
|------------------------|--------------------------------------------------------------------------------------------------------------------------------|
| `set_sample_root_note` | Set the root note (MIDI note number 0–127) for pitch mapping. Accepts note name ("C4") or number (60).                         |
| `set_sample_loop`      | Configure loop region: `enabled`, `start` (seconds), `end` (seconds), `crossfade_ms`. Set `enabled: false` to disable looping. |
| `set_sample_crop`      | Set crop start/end in seconds. Set to `null` to remove crop (use full sample).                                                 |
| `normalize_sample`     | Normalize sample peak level to target dB (default 0 dB). Returns the gain change applied.                                      |
| `reverse_sample`       | Reverse the sample audio data.                                                                                                 |
| `trim_silence`         | Auto-trim silence from start and end based on threshold (default -40 dB). Returns new crop region.                             |

#### Audio Input & Recording

| Tool                 | Description                                                                                                                           |
|----------------------|---------------------------------------------------------------------------------------------------------------------------------------|
| `list_input_devices` | List available audio input devices with channel counts and supported sample rates.                                                    |
| `get_input_state`    | Get current input state: selected device, monitoring status, recording status, peak level.                                            |
| `set_input_device`   | Select an audio input device by name or index.                                                                                        |
| `start_monitoring`   | Enable input monitoring (hear input through output). Returns latency in ms.                                                           |
| `stop_monitoring`    | Disable input monitoring.                                                                                                             |
| `start_recording`    | Start recording from the active input device. Accepts optional `name` for the new sample. Returns immediately (recording runs async). |
| `stop_recording`     | Stop recording and add the captured audio to the sample library. Returns the new `SampleId`, duration, and metadata.                  |

#### Sampler Module Control

| Tool                      | Description                                                                                                                                                   |
|---------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `assign_sample_to_module` | Assign a sample (by id) to a Sampler module (by instrument_id + module_id). This is equivalent to changing the `SampleSelect` parameter.                      |
| `get_sampler_state`       | Get the current state of a Sampler module: assigned sample, pitch tracking, loop mode, direction, start offset, level, velocity sensitivity.                  |
| `set_sampler_parameter`   | Set any Sampler module parameter: `pitch_tracking`, `loop_enabled`, `direction` (forward/reverse/ping_pong), `start_offset`, `level`, `velocity_sensitivity`. |
| `preview_sample`          | Play a sample once through the master output for auditioning. Accepts optional `start` and `end` (seconds) for partial preview.                               |
| `stop_preview`            | Stop any currently playing sample preview.                                                                                                                    |

### 9.2 SynthBridge Extension

Extend the existing `SynthBridge` trait with sample operations:

```rust
// synth_mcp/src/bridge.rs — new methods

pub trait SynthBridge: Send + Sync {
    // ... existing methods ...

    // Sample library
    fn list_samples(&self, filter: Option<&str>) -> Vec<SampleInfo>;
    fn get_sample_info(&self, id: SampleId) -> Option<DetailedSampleInfo>;
    fn import_sample(&self, path: &Path, name: Option<&str>, root_note: Option<MidiNote>) -> Result<SampleInfo>;
    fn delete_sample(&self, id: SampleId, force: bool) -> Result<()>;
    fn rename_sample(&self, id: SampleId, name: &str) -> Result<()>;
    fn duplicate_sample(&self, id: SampleId) -> Result<SampleInfo>;
    fn export_sample(&self, id: SampleId, path: &Path, bit_depth: BitDepth) -> Result<()>;

    // Sample editing
    fn set_sample_root_note(&self, id: SampleId, note: MidiNote) -> Result<()>;
    fn set_sample_loop(&self, id: SampleId, config: LoopConfig) -> Result<()>;
    fn set_sample_crop(&self, id: SampleId, crop: Option<CropRegion>) -> Result<()>;
    fn normalize_sample(&self, id: SampleId, target_db: Decibels) -> Result<Decibels>;
    fn reverse_sample(&self, id: SampleId) -> Result<()>;
    fn trim_silence(&self, id: SampleId, threshold_db: Decibels) -> Result<CropRegion>;

    // Audio input
    fn list_input_devices(&self) -> Vec<InputDeviceInfo>;
    fn get_input_state(&self) -> InputState;
    fn set_input_device(&self, device: &str) -> Result<()>;
    fn start_monitoring(&self) -> Result<Milliseconds>;
    fn stop_monitoring(&self) -> Result<()>;
    fn start_recording(&self, name: Option<&str>) -> Result<()>;
    fn stop_recording(&self) -> Result<SampleInfo>;

    // Sampler module
    fn assign_sample(&self, instrument_id: InstrumentId, module_id: ModuleId, sample_id: SampleId) -> Result<()>;
    fn get_sampler_state(&self, instrument_id: InstrumentId, module_id: ModuleId) -> Result<SamplerState>;
    fn preview_sample(&self, id: SampleId, start: Option<Seconds>, end: Option<Seconds>) -> Result<()>;
    fn stop_preview(&self) -> Result<()>;
}
```

### 9.3 Shared State for GUI Sync

Same pattern as AWE: bidirectional sync between MCP and GUI via `McpSharedState`:

```rust
// pertylizer/src/mcp_bridge.rs — extend McpSharedState

pub struct McpSharedState {
    // ... existing fields ...

    /// Pending sample library changes from MCP (GUI polls and applies).
    pub pending_sample_ops: Mutex<Vec<SampleOp>>,
    /// Current sample library snapshot for MCP reads.
    pub sample_library_snapshot: RwLock<Vec<SampleInfo>>,
    /// Current input state for MCP reads.
    pub input_state: RwLock<InputStateSnapshot>,
}

pub enum SampleOp {
    Import { path: PathBuf, name: Option<String>, root_note: Option<MidiNote> },
    Delete { id: SampleId, force: bool },
    Rename { id: SampleId, name: String },
    SetRootNote { id: SampleId, note: MidiNote },
    SetLoop { id: SampleId, config: LoopConfig },
    SetCrop { id: SampleId, crop: Option<CropRegion> },
    Normalize { id: SampleId, target_db: Decibels },
    Reverse { id: SampleId },
    TrimSilence { id: SampleId, threshold_db: Decibels },
    StartRecording { name: Option<String> },
    StopRecording,
    StartMonitoring,
    StopMonitoring,
    SetInputDevice { device: String },
}
```

### 9.4 AI Descriptions & Validation

Following the AWE MCP pattern:

- **Every tool** gets a detailed description with parameter types, valid ranges, units, and usage examples
- **Every enum value** (PlayDirection, ChannelCount, BitDepth) is listed with descriptions
- **Error messages** are comprehensive: invalid sample id returns available sample ids, invalid parameter returns valid
  range
- **Server instructions** updated with a "Sampling" section explaining the workflow:
    1. Import or record samples
    2. Edit properties (root note, loop, crop)
    3. Add Sampler module to instrument in Rack
    4. Assign sample to Sampler module
    5. Connect Sampler output to signal chain
    6. Play notes to trigger sample playback

### 9.5 Example AI Workflow

```
AI → list_samples()
   ← [{ id: 1, name: "Vocal", duration: 2.3s, root_note: C4 }]

AI → import_sample({ path: "/home/user/kicks/kick01.wav", root_note: "C2" })
   ← { id: 2, name: "kick01", duration: 0.4s, channels: "mono", root_note: C2 }

AI → set_sample_loop({ id: 1, enabled: true, start: 0.5, end: 2.1, crossfade_ms: 12 })
   ← { success: true }

AI → add_module({ instrument_id: 0, module_type: "sampler" })
   ← { module_id: "smp-1" }

AI → assign_sample_to_module({ instrument_id: 0, module_id: "smp-1", sample_id: 1 })
   ← { success: true }

AI → connect_ports({ from: "smp-1:out", to: "flt-1:in" })
   ← { success: true }

AI → play_note({ note: "C4", velocity: 100 })
   ← (sample plays through the patch at original pitch)

AI → start_recording({ name: "Ambient Room" })
   ← { recording: true }
   ... (user speaks / plays) ...
AI → stop_recording()
   ← { id: 3, name: "Ambient Room", duration: 8.2s }
```

---

## 10. Implementation Phases

### Phase 1 — Foundation (sample data + WAV I/O)

**Goal:** Load and save WAV files, core data types.

1. Create `synth_sampler` crate with workspace dependency
2. Implement `SampleId`, `SampleMeta`, `Sample`, `LoopRegion`, `CropRegion`, `FrameIndex`, `PlaybackPosition`,
   `PlaybackSpeed` types (re-export `ChannelCount` and `FrameCount` from `synth_core`; add `Serialize/Deserialize` to
   `FrameCount` first)
3. Implement `SampleLibrary` with add/remove/get/list
4. Implement `load_wav()` and `save_wav()` using `hound`
5. Implement basic sample rate conversion (linear interpolation)
6. Add unit tests for WAV round-trip and library operations

**Deliverable:** Can load WAV files into memory and inspect metadata.

---

### Phase 2 — Sample View (GUI)

**Goal:** Browse, import, edit samples in a dedicated view.

1. Add `AppView::Sample` to navigation
2. Create `gui/sample_view.rs` with the layout described in section 5
3. Implement sample list panel (left sidebar)
4. Implement waveform display widget with zoom and scroll
5. Implement crop handles (drag to set start/end)
6. Implement loop region markers
7. Implement properties panel (name, root note, loop settings)
8. Implement toolbar actions: Import WAV, Export WAV, Delete, Normalize, Reverse, Auto-trim
9. Implement waveform peak cache for rendering performance
10. Wire up `SampleLibrary` to the GUI via `SynthSession`
11. Add sample memory indicator to top bar (next to CPU meter)

**Deliverable:** Fully functional sample browser and editor view.

---

### Phase 3 — Audio Input (recording)

**Goal:** Record audio from microphone/line-in.

1. Add `start_input()` to `AudioBackend` trait (device enumeration already exists via `devices()`)
2. Implement input stream in `CpalBackend` — cpal callback writes to **two** `HeapProd<f32>` (engine + GUI)
3. Create `AudioInputManager` with dual ring buffers, monitoring and recording state machine
4. Add input device selector dropdown to Sample view bottom bar
5. Add level meter showing real-time input level
6. Add Monitor toggle (pass input audio to output)
7. Add Record button: starts capturing to a growing buffer
8. On stop recording: create `Sample` from captured data and add to library
9. Add recording countdown / pre-roll option

**Deliverable:** Can record audio from microphone directly into sample library.

---

### Phase 4 — Sampler Module (in Rack)

**Goal:** Play samples as instruments in the patch editor.

1. Add `ModuleType::Sampler` variant
2. Implement `SamplerParam` enum in `synth_core`
3. Implement `SamplePlayer` DSP with pitch tracking and interpolation
4. Implement `SamplerModule` as `PolyModule` in `synth_modules`
5. Add engine-side sample cache (`HashMap<SampleId, SampleCacheEntry>`)
6. Add `EngineCommand::LoadSample` / `UnloadSample` handling
7. Register in `module_factory.rs`
8. Create module panel UI in Rack view with sample selector and mini waveform
9. Wire NoteOn/NoteOff to sample playback with velocity sensitivity

**Deliverable:** Can build patches using samples as sound sources alongside oscillators.

---

### Phase 5 — Live Audio Input Module

**Goal:** Route live audio input into the patch graph.

1. **Refactor `ProcessContext` to `ProcessContext<'a>`** — add `audio_input: Option<&'a [f32]>` field.
   Update `PolyModule::process()` and `AudioEffect::process()` trait signatures. This is a mechanical
   change touching ~35 existing modules (signature only, no logic change). Do this first as a
   standalone commit.
2. Add `audio_input_buffer: Vec<f32>` and `audio_input_consumer: Option<HeapCons<f32>>` to `SynthEngine`
3. In `SynthEngine::process()`, drain the consumer into `audio_input_buffer` once per block (before voice processing)
4. Pass `audio_input` reference through `ProcessContext` to all voice modules
5. Create `AudioInputModule` (implements `PolyModule`) — reads from `ProcessContext::audio_input`
   instead of the ring buffer directly. All voice instances read the same immutable buffer.
   Only runs in active voices (user must play a note to hear input — same as Noise module).
6. Add `EngineCommand::SetAudioInputConsumer`
7. Module has `"out"` port — can be connected to filters, effects, etc.
8. Add to module factory as a special module (one per instrument max)
9. UI shows input level and "Live" indicator when active
10. Enable recording directly from the Rack view (records the input module's output post-effects)

**Deliverable:** Real-time audio input usable as a signal source in any patch.

---

### Phase 6 — Bundle Project Format

**Goal:** Save/load projects with embedded samples.

1. Add `zip` crate dependency
2. Implement `save_bundle()` — ZIP with project.json + samples/ + metadata.json
3. Implement `load_bundle()` — extract and load samples into library
4. Auto-detect format in `load_file()` (ZIP magic bytes vs JSON)
5. Update `ProjectFile` with `sample_refs` field
6. Implement patch bundle format (`.pertpatch`) for single instruments with samples
7. Handle sample deduplication (same sample used by multiple instruments)
8. Migration: opening a v1 project still works, saving creates v2 bundle

**Deliverable:** Projects with samples save/load as self-contained bundles.

---

### Phase 7 — MCP Integration

**Goal:** Full remote control of sampling via MCP (see [section 9](#9-mcp-integration-full-remote-control) for details).

1. Extend `SynthBridge` trait with all sample library methods
2. Implement `list_samples`, `get_sample_info`, `import_sample`, `delete_sample`, `rename_sample`, `duplicate_sample`,
   `export_sample` tools
3. Implement sample editing tools: `set_sample_root_note`, `set_sample_loop`, `set_sample_crop`, `normalize_sample`,
   `reverse_sample`, `trim_silence`
4. Implement audio input tools: `list_input_devices`, `get_input_state`, `set_input_device`, `start_monitoring`,
   `stop_monitoring`, `start_recording`, `stop_recording`
5. Implement sampler module tools: `assign_sample_to_module`, `get_sampler_state`, `set_sampler_parameter`,
   `preview_sample`, `stop_preview`
6. Extend `McpSharedState` with `pending_sample_ops` and `sample_library_snapshot` for bidirectional GUI sync
7. Add rich AI descriptions with parameter ranges, units, and usage hints to all tools
8. Update MCP server instructions with "Sampling" workflow section
9. Test full AI workflow: import → edit → assign → play

**Deliverable:** AI agents can fully manage samples, record audio, and build sampler instruments remotely.

---

### Phase 8 — Polish & Integration

**Goal:** Tie everything together, edge cases, UX refinement.

1. Sample preview in the Rack view (click sample name to audition)
2. Drag-and-drop: drag WAV files from OS file manager into Sample view
3. Undo/redo for sample edits (crop, normalize, reverse)
4. Sample usage tracking (show which instruments use each sample)
5. Warning on delete if sample is in use
6. Performance testing with large samples (>10 minutes)
7. Memory management: unload unused samples, streaming for very large files

---

## 11. Important Features You May Have Missed

These are features that are critical for a usable sampling workflow but weren't explicitly mentioned:

### 11.1 Root Note / Pitch Mapping

Every sample needs a **root note** (the pitch at which the sample plays back at its original speed). Without this, pitch
tracking is impossible. The Sampler module uses: `speed = 2^((played_note - root_note) / 12.0)`.

### 11.2 Sample Rate Conversion

Samples may be recorded at 48kHz but the engine runs at 44.1kHz (or vice versa). Conversion must happen on import, not
during real-time playback.

### 11.3 Crossfade at Loop Points

Without crossfade, looped samples will click at the loop boundary. The `LoopRegion::crossfade` field is essential — it
blends the end into the start of the loop over N frames.

### 11.4 Anti-aliasing During Pitch Shifting

Cubic Hermite interpolation (used in `SamplePlayer`) is effective against *imaging* when pitching
**down** (speed < 1.0). However, when pitching **up** (speed > 1.0), high frequencies in the
sample fold back below Nyquist as aliasing (metallic artifacts). A proper solution requires a
dynamic low-pass filter (cutoff = Nyquist / speed) applied before interpolation. For MVP, cubic
interpolation is sufficient and sounds "classic sampler"-like. Add the anti-alias LP filter as a
future improvement if users report metallic artifacts on dark samples played at high notes.

### 11.5 One-Shot vs Sustain Playback Modes

- **One-shot**: plays the full sample regardless of NoteOff (drums, percussion)
- **Sustain**: plays until NoteOff, then enters release phase (pads, sustained sounds)
- **Loop**: plays to loop end, loops back to loop start until NoteOff

### 11.6 Amplitude Envelope Integration

The Sampler module should work with the existing envelope modules. Its output connects to the Amplifier module via the
existing patch graph, so the envelope shapes the sample's volume naturally.

### 11.7 Gain / Normalization

Recorded samples will have different levels. Need a normalize function and a per-sample gain setting to balance levels
before they hit the patch graph.

### 11.8 Monitoring Latency Warning

When "Monitor" is enabled, the user hears their input through the output with the audio buffer latency. If latency is >
20ms, show a warning. Consider offering a "direct monitoring" bypass option if the audio interface supports it.

### 11.9 Silence Detection / Auto-Trim

When recording, it's common to have silence at the beginning and end. Offer an "auto-trim" that sets crop markers at the
first/last sample above a threshold (e.g., -40 dB).

### 11.10 Memory Pressure

Each second of stereo 48kHz audio is ~384 KB. A project with many long samples could use hundreds of megabytes. Need to:

- Show sample memory usage in the **top bar** next to the CPU meter (e.g., "Mem: 12.4 MB"), same style and position as
  the existing CPU/Voices indicators
- Also show total in the sample list panel footer (e.g., "5 samples — 12.4 MB")
- Consider streaming from disk for samples >30 seconds (future optimization)

---

## 12. Future Feature Ideas

These are not part of the initial implementation but worth considering for later:

### 12.1 Multi-sample Instruments

Map different samples to different key ranges and velocity layers (like a piano with separate recordings per octave).
This is the standard approach for realistic acoustic instruments.

### 12.2 Sample Slicing

Automatically or manually slice a long sample (e.g., a drum loop) into individual hits mapped to consecutive MIDI notes.
Essential for beat-making workflows.

### 12.3 Timestretch

Change the tempo/duration of a sample without changing its pitch. Requires a phase vocoder or WSOLA algorithm. Already
have `PhaseVocoder` in effects — could adapt.

### 12.4 Granular Sampler

Extend the existing `GranularOsc` to use loaded samples as grain sources instead of only synthetic waveforms. This would
be very powerful — the `GrainSource` enum already exists, just add a `Sample(SampleId)` variant.

### 12.5 Audio Track in Sequencer

Place samples directly on the sequencer timeline as audio clips alongside MIDI patterns. Requires significant sequencer
engine changes.

### 12.6 Sample Recording from Engine Output

"Resample" — record the output of an instrument as a new sample. Useful for capturing a complex patch as a simple sample
for CPU savings.

### 12.7 External Sample Library Browsing

Browse and preview samples from a user's sample library folder (e.g., Splice, sample packs) without importing everything
into the project.

### 12.8 Auto-tune / Pitch Detection

Detect the pitch of a recorded sample and auto-set the root note. Useful for vocal recordings.

### 12.9 Non-destructive Editing

Keep the original sample data and stack operations (crop, normalize, reverse, fade) as non-destructive edits. Allows
unlimited undo.

### 12.10 AIFF and Other Format Support

Currently only WAV. Could add AIFF, FLAC, OGG via additional crates.

### 12.11 Convolver Integration

Allow loading user WAV files as impulse responses for the existing Convolver effect (currently only synthetic IRs). The
Sample Library could serve as the IR source.

### 12.12 Vocal Processing Chain

If voice recording is a primary use case, consider adding a pre-built vocal chain: noise gate → compressor → EQ →
reverb, configurable as a "vocal preset" in the Sampler module.

### 12.13 Punch-in Recording

Record over a specific section of an existing sample while preserving the rest. Common in vocal recording workflows.

### 12.14 Sample Markers / Cue Points

Add named markers within a sample (e.g., "Verse 1", "Chorus") for quick navigation and as alternate start points.

### 12.15 Batch Import

Import an entire folder of WAV files at once, auto-naming from filenames.

### 12.16 Sample Preview Before Import

When browsing WAV files in the import dialog, allow previewing/auditioning the file before committing to import.
Requires a lightweight file-based player that bypasses the sample library.

### 12.17 Zero-Crossing Snap

When dragging crop or loop markers in the Sample view, offer a "Snap to Zero-Crossing" mode that
automatically finds the nearest point where the waveform crosses 0.0. This makes loops and crops
click-free without needing crossfade, and is especially useful for single-cycle waveforms.

### 12.18 Convert to Wavetable

Since `WavetableOsc` already exists, add a function in the Sample view to extract a single waveform
cycle from a looped sample and export it as a wavetable that `WavetableOsc` can load. Enables
creating custom wavetables from any recorded or imported audio.

### 12.19 Disk Streaming for Large Files

The current `Arc<[f32]>` design loads everything into RAM — great for performance but a 10-minute
stereo 48kHz backing track uses ~115 MB. For samples exceeding ~30 seconds, consider a `mmap`-based
or streaming reader that pages data from disk on demand, keeping only the active playback window
in memory.

### 12.20 Drum Rack / Multi-pad Instrument

A dedicated module UI (similar to Ableton's Drum Rack) where 16 pads each map to a separate sample
with independent output routing, volume, pan, and mini effect chains. Each pad triggers on a fixed
MIDI note (C1–D#2 by default). More specialized than the generic multi-sample mapping in 12.1.
