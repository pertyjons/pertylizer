# Project Instructions for Pertylizer

## Language

All code, comments, UI strings, documentation, and commit messages in **English**.

## Project Phase

Active development — **no backward compatibility required**. Break APIs freely.

## Commands

### `git commit`

```bash
git add --all
git commit -m "<short description of changes>"
```

### `new version`

1. Update `docs/history.md` with new version number and changes
2. Review `docs/TODO.md` and mark completed tasks
3. Update version number in `Cargo.toml`

---

## Architecture

### Crate Structure

| Crate             | Purpose                                                            |
|-------------------|--------------------------------------------------------------------|
| `pertylizer`      | GUI application (egui/eframe), MCP integration, project management |
| `synth_core`      | Shared types, traits (`PolyModule`, `ModuleDescriptor`), newtypes  |
| `synth_engine`    | Audio engine, voice management, instrument graph, recording        |
| `synth_sequencer` | Song, patterns, tracks, arrangement, automation                    |
| `synth_modules`   | DSP module implementations (oscillators, filters, envelopes, etc.) |
| `synth_dsp`       | Low-level DSP primitives (biquad, delay lines, interpolation)      |
| `synth_awe`       | Acoustic World Engine (room simulation)                            |
| `synth_mcp`       | MCP server for external control                                    |
| `synth_osc`       | OSC protocol support                                               |

### Thread Model

- **Audio thread** — real-time, lock-free. Runs `SynthEngine::process()`. Communicates via `EngineCommand` (in) and
  `EngineEvent` (out) ring buffers.
- **UI thread** — egui rendering. Holds `EngineHandle` for sending commands and reading shared atomic state.
- **Shared state** — `Arc<RwLock<Song>>` for sequencer data. Audio thread uses `try_read()` only. UI thread uses
  `write()` for mutations. Collect snapshots before rendering, release lock, then draw.

### GUI Architecture (egui)

- Icons: `egui_remixicon::icons as ri` (Remix Icon font)
- Panel order: TopPanel → SidePanel → TopBottomPanel::bottom → CentralPanel (last)
- Patch editor modules use `egui::Area` at `Order::Background`. Keyboard panel renders at `Order::Middle` for input
  priority.
- Pattern data collected as snapshots (`collect_arrangement_data`, `collect_piano_roll_data`) before rendering to
  minimize lock hold time.

---

## Newtype Pattern (CRITICAL)

**NEVER use raw primitives** for domain concepts. ALWAYS wrap in a newtype.

```rust
// WRONG — raw primitives for domain values
fn set_frequency(hz: f32) { ... }

// RIGHT — newtypes
fn set_frequency(freq: Hertz) { ... }
```

**Raw primitives OK for:** loop counters, intermediate arithmetic, FFI/serialization internals.

**Search the codebase first** — a suitable newtype likely exists:

| Crate             | Examples                                                                                                                                                                                                      | 
|-------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `synth_core`      | `Hertz`, `SampleRate`, `Cents`, `Semitones`, `MidiNote`, `Velocity`, `Gain`, `Decibels`, `Seconds`, `Milliseconds`, `Bpm`, `NormalizedValue`, `BipolarValue`, `Phase`, `SampleCount`, `BlockSize`, `PortName` |
| `synth_sequencer` | `PatternId`, `TrackId`, `NoteId`, `Tick`, `PatternTick`, `Duration`, `Pitch`, `TrackIndex`, `RowIndex`, `SeqInstrumentId`                                                                                     |
| `synth_engine`    | `TransactionId`, `ClientId`, `InstrumentId`, `MidiChannel`, `ConnectionCount`, `ModuleId`                                                                                                                     |
| `synth_awe`       | `Meters`, `SquareMeters`, `CubicMeters`, `SampleOffset`, `StretchFactor`                                                                                                                                      |

---

## Code Style

- Use `Self` in impl blocks, not the type name
- `thiserror` for error types — no manual `Display + Error` impls
- No `.unwrap()` / `.expect()` in production code — use `unwrap_or`, `?`, or `if let`
- `pub(crate)` for internal types — minimize public API surface
- `#[must_use]` on newtypes and builder methods
- No `unsafe` code without discussion

---

## Build & Code Quality

ALL must pass with **zero warnings or errors**:

```bash
cargo build                  # RUSTFLAGS="-D warnings" in .cargo/config.toml
cargo clippy --all-targets   # Lints configured in Cargo.toml
cargo test
cargo fmt --check
```

Allowed clippy exceptions: `too_many_lines` (large `process()` functions), `cast_precision_loss` (usize → f32 in audio),
`cast_possible_truncation` (value guaranteed to fit).

---

## Real-Time Safety (audio thread)

In `process()` functions and real-time-critical code:

**Forbidden:** heap allocations (`Vec::push`, `Box::new`, `String::clone`), blocking locks (`Mutex::lock`,
`RwLock::write`), panics (`unwrap()`, `expect()`, out-of-bounds indexing), system calls (file I/O, logging).

**Allowed:** `unwrap_or(0.0)` for safe defaults, pre-allocated buffers, atomics, lock-free structures.

**For-loops** for DSP sample processing. **Iterators** outside the hot path.
