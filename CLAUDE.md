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

### Newtype Pattern (mandatory)

ALWAYS use typed domain values — **never raw primitives** like `f32`, `u8`, `usize` for domain concepts.

```rust
// WRONG:
fn set_frequency(hz: f32) { ... }
fn set_cutoff(hz: f32) { ... }  // Easy to mix up!

// RIGHT:
fn set_frequency(freq: Hertz) { ... }
fn set_cutoff(cutoff: Hertz) { ... }  // Type-safe
```

Existing domain types:
- **Frequency:** `Hertz`
- **Amplitude:** `Gain`, `Decibels`
- **Time:** `Seconds`, `Milliseconds`, `Bpm`, `BeatDivision`
- **Normalized:** `NormalizedValue` (0.0–1.0), `BipolarValue` (-1.0 to 1.0), `Phase`
- **MIDI:** `MidiNote`, `MidiChannel`
- **Samples:** `SampleCount`, `SamplePosition`, `SampleRate`, `BufferIndex`
- **DSP:** `FilterState`, `NoiseState`

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