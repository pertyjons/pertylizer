# Project Instructions for Pertylizer

## Language

This project uses **English** for all code, comments, UI strings, documentation, and commit messages.

## Project Phase

Active development — **no backward compatibility required**. Break APIs freely to improve the code.

## Commands

### `git commit`
Stage all files (new and changed) and commit with a short description:
```bash
git add --all
git commit -m "<short description of changes>"
```

### `new version`
1. Update `docs/history.md` with new version number and changes since last version
2. Review `docs/TODO.md` and mark completed tasks as done
3. Update version number in `Cargo.toml`

---

## Code Style & Patterns

### Newtype Pattern (CRITICAL — strictly enforced)

**NEVER use raw primitives** (`f32`, `u8`, `u16`, `u32`, `u64`, `usize`, `i32`) for domain concepts. ALWAYS use or create a newtype wrapper. This applies everywhere: function parameters, return types, struct fields, local variables that represent a domain concept.

**Before writing `f32` or `u8` etc. in a struct field or function signature, STOP and check if a newtype already exists below.** If one exists, use it. If none fits, create a new newtype.

```rust
// WRONG — raw primitives for domain values:
fn set_frequency(hz: f32) { ... }
struct ClipboardNote { pitch: u8, velocity: f32 }
fn quantize(tick: u32, grid: u32) { ... }

// RIGHT — newtypes:
fn set_frequency(freq: Hertz) { ... }
struct ClipboardNote { pitch: Pitch, velocity: Velocity }
fn quantize(tick: PatternTick, grid: Duration) { ... }
```

**The only acceptable uses of raw primitives are:**
- Loop counters and array indices (not representing domain concepts)
- Intermediate arithmetic inside a function that returns a newtype
- FFI boundaries and serialization internals

#### Domain types by crate

**`synth_core`** (30 types):
- **Frequency:** `Hertz`, `SampleRate` (f32, for DSP)
- **Pitch:** `Cents`, `Semitones`, `Octaves`, `MidiNote`, `Velocity`, `MidiChannel`
- **Amplitude:** `Gain`, `Decibels`, `Ratio`, `Amplitude`
- **Time:** `Seconds`, `Milliseconds`, `Bpm`, `BeatDivision`, `BeatPosition`
- **Normalized:** `NormalizedValue` (0.0–1.0), `BipolarValue` (-1.0–1.0), `Phase` (0.0–1.0)
- **Samples:** `SampleCount`, `SamplePosition`, `BlockSize`
- **Audio:** `BufferIndex`, `FrameCount`, `NoiseState`, `FilterState`, `VoiceCount`, `CpuUsage`, `PatternIndex`
- **Interned:** `PortName`
- **Audio backend:** `SampleRate` (u32), `BufferSize`

**`synth_sequencer`** (13 types):
- **IDs:** `PatternId`, `TrackId`, `SeqInstrumentId`, `NoteId`
- **Indices:** `TrackIndex`, `RowIndex`, `TrackCount`, `RowCount`, `TicksPerRow`
- **Time:** `Tick` (absolute), `PatternTick` (pattern-local), `Duration` (in ticks)
- **Pitch:** `Pitch` (0–127)

**`synth_engine`** (5 types):
- `TransactionId`, `ClientId`, `InstrumentId`, `MidiChannel`, `ConnectionCount`

**`synth_awe`** (6 types):
- **Physical:** `Meters`, `SquareMeters`, `CubicMeters`, `MetersPerSecond`
- **Audio:** `SampleOffset`, `StretchFactor`

### Naming Conventions

- **Types:** `PascalCase` — `Hertz`, `NormalizedValue`
- **Functions/methods:** `snake_case` — `to_frequency()`, `as_f32()`
- **Constants:** `SCREAMING_SNAKE_CASE` — `Hertz::A4`, `Gain::UNITY`
- **Use `Self`** in impl blocks, not the type name

---

## Build & Code Quality

### Required Checks

Before a task is considered done, ALL of the following must pass with zero warnings or errors:

```bash
# Step 1: Compile (RUSTFLAGS="-D warnings" configured in .cargo/config.toml)
cargo build

# Step 2: Clippy (lints configured in Cargo.toml)
cargo clippy --all-targets

# Step 3: Run all tests
cargo test

# Step 4: Check formatting
cargo fmt --check
```

### Strict Rules

1. **No `.unwrap()` or `.expect()` in production code** — use `unwrap_or`, `unwrap_or_default`, `?`, or `if let`
2. **No `unsafe` code** — discuss first if absolutely necessary
3. **`pub(crate)`** for internal types where reasonable
4. **`#[must_use]`** on newtypes, builder methods, and functions returning values that shouldn't be ignored
5. **`thiserror`** for all error types — no manual `Display + Error` impls

### Allowed Exceptions

These clippy allows are OK:
```rust
#[allow(clippy::too_many_lines)]           // On large process() functions
#[allow(clippy::cast_precision_loss)]      // usize -> f32 in audio
#[allow(clippy::cast_possible_truncation)] // Where value is guaranteed to fit
```

`.unwrap()` and `.expect()` are allowed in:
- Tests
- One-time initializations guaranteed to succeed (e.g., regex, constants)

---

## Real-Time Safety (audio thread)

In `process()` functions and other real-time-critical code:

### Forbidden
- **Heap allocations:** `Vec::push`, `HashMap::insert`, `String::clone`, `Box::new`
- **Blocking locks:** `Mutex::lock`, `RwLock::write`
- **Panics:** `unwrap()`, `expect()`, `panic!`, out-of-bounds indexing

### Allowed
- `unwrap_or(0.0)` for safe sample defaults
- Pre-allocated buffers
- Atomic operations
- Lock-free structures

### For-loops vs Iterators

**Keep for-loops** in audio DSP for sample processing:
```rust
for i in 0..samples {
    output[i] = input[i] * gain;
}
```

**Use iterators** outside the hot path for collection operations.