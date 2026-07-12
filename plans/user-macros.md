# User-assignable macro knobs

Status: PROPOSED

Origin: external architecture review (§5, "Makro-kontroller"), evaluated against
the code 2026-07-12. A genuine gap — the second strongest point in the review.

## 1. Current state (verified)

The mod matrix has "macro" **sources**, but they are six fixed
performance/MIDI inputs — not free, user-assignable knobs:

```rust
// synth_core/src/params/mod_matrix.rs:610
/// A *true macro* source — a named per-voice scalar with no `ModuleId`.
/// The six decided macros.
pub enum MacroSource { Velocity, NoteNumber, Aftertouch, ModWheel, PitchBend, PolyAftertouch }
```

`SrcAddr::Macro(MacroSource)` (`mod_matrix.rs:660`) feeds a routing; the voice
fills these once per block into `MacroValues` (`voice.rs:53`, built at
`voice.rs:1074`) and `Voice::resolve_source` maps each variant to its field
(`voice.rs:1204`). There is a GUI "macro rail" (S1.5b) that displays these six.

What's missing: a bank of **named, patch-authored knobs** (e.g. `Macro 1
"Wobble"`, `Macro 2 "Darkness"`) that a user turns to drive dozens of
destinations through the mod matrix with one gesture. Today the closest
approximation is routing `ModWheel` to many destinations — one hijacked
controller, unnamed, and only one of it.

## 2. Goal

Add N user macros (propose **8**) that are:

- **Named** and saved with the patch.
- Adjustable from the **GUI** (a knob rail), **MCP** (`set_macro`), and as
  **automation lane** targets in the sequencer.
- Usable as a **mod-matrix source** (`SrcAddr::Macro(UserMacro(i))` →
  `macro-1` … `macro-8`), so one knob fans out to many destinations, each with
  its own bipolar `amount`.

## 3. Design

### A. Source model

Extend the macro source with a user variant:

```rust
pub enum MacroSource {
    Velocity, NoteNumber, Aftertouch, ModWheel, PitchBend, PolyAftertouch,
    User(u8),   // 0..MACRO_COUNT
}
```

- Stable ids `"macro1"`..`"macro8"` in `id()` / `from_id()` so routings
  round-trip through the `slot_N_source` serialization.
- `Voice::resolve_source` (`voice.rs:1204`) gains a `User(i)` arm reading
  `macros.user[i]`.
- `MacroValues` (`voice.rs:53`) grows `user: [f32; MACRO_COUNT]`, filled per
  block from the current macro bank (block-constant — RT-safe, no allocation).

### B. Storage / ownership — the key decision

A Pertylizer "patch" is one **instrument**, and each instrument has its own
graph + mod matrix. So macros should be **per-instrument** (each instrument
carries its own bank), matching the mod matrix's scope. A separate future
"song-global macro" concept is possible but out of scope here.

Persist a bank in the instrument's saved state:

```rust
struct MacroDef { name: String, value: f32 /* 0..1 or bipolar — see open Q */ }
struct MacroBank { macros: [MacroDef; MACRO_COUNT] }
```

Feed the live values to the engine so the voice can read them per block (same
side-channel pattern as other per-instrument scalars; the value updates on the
GUI/MCP thread, the audio thread reads a snapshot).

### C. GUI

Extend the existing macro rail (S1.5b) with the 8 user knobs: a knob + editable
name label each. Turning a knob updates the bank; the name is persisted and
shown as the source label in the mod-matrix picker and on source markers.

### D. MCP

- `set_macro(instrument, index, value)` — array-capable (per house style).
- `set_macro_name(instrument, index, name)`.
- Surface macros in `get_instrument_info` and as sources in
  `get_mod_matrix_routings` / the source enumeration.

### E. Automation

Macros are plain scalars — expose them as automation-lane targets so a macro
sweep can itself be automated (macro → many dests → one automation lane drives
the lot).

## 4. Real-time safety

Macro values are block-constant per-instrument scalars, snapshotted for the
audio thread and read once per block into `MacroValues`. No allocation, no lock
in `process()` — identical discipline to the existing six macros.

## 5. Files to touch

- `crates/synth_core/src/params/mod_matrix.rs` — `MacroSource::User`, ids,
  `MACRO_COUNT`.
- `crates/synth_engine/src/voice.rs` — `MacroValues.user`, fill + resolve arm.
- `crates/synth_engine/` — per-instrument macro bank on the instrument, command
  to update value/name, audio-thread snapshot.
- `crates/pertylizer/src/patch.rs` — serialize the macro bank (names + values).
- MCP (`synth_mcp` / `mcp_bridge.rs`) — `set_macro`, `set_macro_name`, source
  listing.
- GUI macro rail — 8 named user knobs + picker/marker labels.

## 6. Open questions

- **Unipolar vs bipolar knob.** A 0..1 macro with per-routing bipolar `amount`
  already gives both directions; recommend **unipolar 0..1** knobs and lean on
  the routing `amount` for polarity/scale.
- **Count.** 8 proposed; confirm against rail layout. Const `MACRO_COUNT` so it's
  one place to change.
- **Per-instrument vs song-global.** Recommend per-instrument (matches mod-matrix
  scope). Revisit a global bank later if cross-instrument macros are wanted.
- **Legacy upgrade.** None needed — this is additive; old patches simply have an
  empty/default bank.

## 7. Exit gate

- 8 named macro knobs per instrument, saved and reloaded with the patch.
- `Macro 1` selectable as a mod-matrix source; one knob drives several
  destinations, each with independent `amount`.
- Settable via MCP and drivable from an automation lane.
- Renaming a macro updates its label everywhere it appears (rail, picker,
  markers).
- Workspace green (`build` / `clippy --all-targets` / `test` / `fmt --check`).
