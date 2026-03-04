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

## Code Style & Patterns

### Newtype Pattern (CRITICAL — strictly enforced)

**NEVER use raw primitives** (`f32`, `u8`, `u16`, `u32`, `u64`, `usize`, `i32`) for domain concepts. ALWAYS wrap them in a newtype. This applies to: function parameters, return types, struct fields, and local variables representing domain concepts.

**Rule of thumb:** If the value has a *unit* or *meaning* beyond "just a number", it MUST be a newtype.

```rust
// WRONG:
fn set_frequency(hz: f32) { ... }
struct ClipboardNote { pitch: u8, velocity: f32 }

// RIGHT:
fn set_frequency(freq: Hertz) { ... }
struct ClipboardNote { pitch: Pitch, velocity: Velocity }
```

**Raw primitives are ONLY acceptable for:**
- Loop counters and array indices (not domain concepts)
- Intermediate arithmetic inside a function that returns a newtype
- FFI boundaries and serialization internals

**Before adding a new primitive, ALWAYS search the codebase first** — a suitable newtype likely already exists. If none fits, create one. The list below shows *examples* of existing types, not a complete inventory:

#### Existing domain types (examples, not exhaustive)

| Crate | Examples |
|-------|----------|
| `synth_core` | `Hertz`, `SampleRate`, `Cents`, `Semitones`, `MidiNote`, `Velocity`, `Gain`, `Decibels`, `Seconds`, `Milliseconds`, `Bpm`, `NormalizedValue`, `BipolarValue`, `Phase`, `SampleCount`, `BlockSize`, `PortName` |
| `synth_sequencer` | `PatternId`, `TrackId`, `NoteId`, `Tick`, `PatternTick`, `Duration`, `Pitch`, `TrackIndex`, `RowIndex` |
| `synth_engine` | `TransactionId`, `ClientId`, `InstrumentId`, `MidiChannel`, `ConnectionCount` |
| `synth_awe` | `Meters`, `SquareMeters`, `CubicMeters`, `SampleOffset`, `StretchFactor` |

### Naming Conventions

- **Types:** `PascalCase` — `Hertz`, `NormalizedValue`
- **Functions/methods:** `snake_case` — `to_frequency()`, `as_f32()`
- **Constants:** `SCREAMING_SNAKE_CASE` — `Hertz::A4`, `Gain::UNITY`
- **Use `Self`** in impl blocks, not the type name

### Error Handling

- **`thiserror`** for all error types — no manual `Display + Error` impls
- Return `Result<T, E>` with descriptive error variants, not stringly-typed errors
- No `.unwrap()` or `.expect()` in production code — use `unwrap_or`, `unwrap_or_default`, `?`, or `if let`
- `.unwrap()`/`.expect()` allowed in: tests, one-time init guaranteed to succeed

### Code Organization

- **`pub(crate)`** for internal types where reasonable — minimize public API surface
- **`#[must_use]`** on newtypes, builder methods, and functions whose return values shouldn't be ignored
- **No `unsafe` code** — discuss first if absolutely necessary
- Prefer composition over deep inheritance-like trait hierarchies
- Keep modules focused — split when a file exceeds ~500 lines

---

## Build & Code Quality

### Required Checks

Before a task is done, ALL must pass with **zero warnings or errors**:

```bash
cargo build                  # RUSTFLAGS="-D warnings" in .cargo/config.toml
cargo clippy --all-targets   # Lints configured in Cargo.toml
cargo test
cargo fmt --check
```

### Allowed Clippy Exceptions

```rust
#[allow(clippy::too_many_lines)]           // Large process() functions
#[allow(clippy::cast_precision_loss)]      // usize -> f32 in audio
#[allow(clippy::cast_possible_truncation)] // Value guaranteed to fit
```

---

## Testing

- Test public behavior, not implementation details
- Use descriptive test names: `test_velocity_clamps_to_valid_range`, not `test1`
- Prefer small focused tests over large integration tests
- Use `assert_eq!` / `assert_ne!` with meaningful messages for non-obvious comparisons

---

## Real-Time Safety (audio thread)

In `process()` functions and real-time-critical code:

### Forbidden
- **Heap allocations:** `Vec::push`, `HashMap::insert`, `String::clone`, `Box::new`
- **Blocking locks:** `Mutex::lock`, `RwLock::write`
- **Panics:** `unwrap()`, `expect()`, `panic!`, out-of-bounds indexing
- **System calls:** file I/O, logging, printing

### Allowed
- `unwrap_or(0.0)` for safe sample defaults
- Pre-allocated buffers
- Atomic operations and lock-free structures

### For-loops vs Iterators

**For-loops** in audio DSP sample processing:
```rust
for i in 0..samples {
    output[i] = input[i] * gain;
}
```

**Iterators** outside the hot path for collection operations.

---

## GUI (egui)

- Keep UI code separate from business logic
- Avoid allocations in per-frame rendering where possible
- Use `egui::Id` for stable widget identity across frames
