# Project Instructions for Pertylizer

## Language

All code, comments, UI strings, documentation, and commit messages in **English**.

## Project Phase

Active development — **no backward compatibility required**. Break APIs freely.

## Commands

### `git commit`

Before committing, ensure the working tree is clean:

```bash
cargo fmt --check
cargo build
cargo clippy --all-targets
cargo test
```

Then:

```bash
git add --all
git commit -m "<short description of changes>"
```

### Merging a branch to `main`

When work was done on a branch, **always squash-merge** it into `main` so each
feature lands as a single clean commit (the branch's incremental commits are not
kept in `main`'s history):

```bash
git checkout main
git merge --squash <branch>
git commit -m "<summary of the whole branch>"
git branch -D <branch>
```

Never fold a working branch into `main` with a plain/ff merge — squash by
default. (`git merge` without `--squash` does a ff/merge-commit, so `--squash`
must be explicit.)

### `new version`

1. **HARD REQUIREMENT — document every change since the last version.**
   `docs/history.md` MUST contain an entry covering **every commit made since the
   previous version's entry**. No commit may be left undocumented. The boundary is
   the most recent release commit, **not** a git tag (versions are tagged only at
   actual releases, so tags lag behind `docs/history.md`). Enumerate the commits
   with `git log "$(git log --grep='^Release v' --format=%H -n 1)"..HEAD --oneline`
   and fold each into the new entry. Cutting a version without doing this is not
   allowed.
2. Add the new version entry to `docs/history.md` (newest on top,
   `## [x.y.z] - YYYY-MM-DD`).
3. Review `plans/TODO.md` and mark completed tasks.
4. Update the version number in `crates/pertylizer/Cargo.toml`, then run
   `cargo build` so `Cargo.lock` is synced.
5. Commit the bump to `main` as `Release vX.Y.Z: <summary>` (follow the
   `git commit` checklist above).
6. **Tag and push — this is what publishes the release:**
   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

### Releases (GitHub Actions)

`.github/workflows/build.yml` triggers **only on pushed `v*` git tags** — never
on plain commits. So you can push as many small version-less commits to `main`
as you like without triggering anything; a release happens **only** when you push
a tag.

When a `v*` tag is pushed the workflow builds Linux/macOS/Windows, then the
`release` job publishes a GitHub release. Key facts:

- **The tag name is the source of truth for the version.** The release version is
  derived from the tag (`v0.313.0` → `0.313.0`), *not* from `Cargo.toml`. Always
  tag with the exact version you bumped `Cargo.toml` to.
- **The tag must point at a commit that already contains** the bumped
  `Cargo.toml` and the matching `## [x.y.z]` entry in `docs/history.md` — the
  release notes are extracted from that section. Tag *after* the `new version`
  commit is on `main`.
- There is no manual (`workflow_dispatch`) trigger; the only way to run a build or
  release is to push a tag.

### `docs/history.md` style

Keep each fix / change description to **one or two sentences** — what was
broken (or added) and the one-line essence of the fix. No multi-paragraph
explanations, no full design rationale, no test-list enumerations. The
commit and the code carry the detail; history.md is the index.

---

## Architecture

### Crate Structure

| Crate                | Purpose                                                            |
|----------------------|--------------------------------------------------------------------|
| `pertylizer`         | GUI application (egui/eframe), MCP integration, project management |
| `synth_core`         | Shared types, traits (`PolyModule`, `ModuleDescriptor`), newtypes  |
| `synth_config`       | Runtime config (`pertylizer.toml`) shared by the app and visualizer |
| `synth_engine`       | Audio engine, voice management, instrument graph, recording        |
| `synth_sequencer`    | Song, patterns, tracks, arrangement, automation                    |
| `synth_modules`      | DSP module implementations (70 module types incl. 25 effects)      |
| `synth_dsp`          | Low-level DSP primitives (biquad, delay lines, interpolation)      |
| `synth_sampler`      | Sample loading, playback, and waveform analysis                    |
| `synth_awe`          | Acoustic World Engine (room simulation)                            |
| `synth_mcp`          | MCP server for external control (175+ tools)                       |
| `synth_osc`          | OSC telemetry sender (spectrum, notes, transport over UDP)         |
| `synth_osc_protocol` | Shared OSC protocol definitions for synth and visualizer           |

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

### Shared widgets (`widgets/controls.rs`)

- **Small reusable widgets and composite controls live in
  `crates/pertylizer/src/gui/widgets/controls.rs`** (themed labels, icon buttons,
  toggles, drag/slider presets, layout idioms like `right_aligned_row`, …). Don't
  hand-roll the same `egui::Button`/`Label`/layout primitive inline at a call
  site when it's a repeatable idiom — add a helper there and call it.
- **Add it even when there's a single caller today.** Putting it in `controls.rs`
  keeps the widget surface easy to consolidate: a later second user reuses it
  instead of diverging, and related helpers can be merged in one place.
- **Scope: small widgets only — not larger composites.** Big, view-specific
  composites (whole panels, the `ModuleFrame` chrome / `draw_module_header` /
  `draw_module_footer` painters in `widgets/frame.rs`, etc.) stay with their view
  or chrome module; those *call* the small `controls.rs` helpers, they don't move
  into `controls.rs`.

---

## MCP Integration

This project includes an MCP (Model Context Protocol) server for external control and inspection.

### Connection

- **HTTP Endpoint:** `http://127.0.0.1:9850/mcp`
- **Gemini CLI Configuration:** Local settings are stored in `.gemini/settings.json`.
- **Claude Code Configuration:** Configuration is in `.mcp.json`.

### Available Tools

The `synth` MCP server provides a wide range of tools:
- **Discovery:** `list_module_types`, `get_module_type_info`, `search_modules`, `list_port_types`.
- **Instruments:** `create_instrument`, `build_instrument`, `list_instruments`, `set_parameter`.
- **Sequencer:** `create_pattern`, `add_note`, `create_track`, `place_pattern`.
- **AWE:** `set_awe_preset`, `set_awe_parameter`, `get_awe_state`.
- **Analysis:** `analyze_harmony`, `analyze_mix_bus`, `analyze_section`.

### Batch Operations

Use `batch_execute` to run multiple operations in a single request for better performance.

### MCP feedback (ALWAYS)

**Scope: this applies ONLY to the `synth` MCP server (Pertylizer's own server,
`http://127.0.0.1:9850/mcp`).** Do not apply it to any other MCP server (e.g. the
`egui` inspection MCP or any unrelated tools) — gaps there are not ours to harden.

We are actively hardening Pertylizer's MCP, so **every time you use a `synth` MCP tool,
watch for gaps and report them.** Whenever a `synth` call errors, returns something
unexpected, is awkward to use, lacks a tool you needed, or has a confusing/incomplete
schema or description — **stop and report it to the user**, then investigate what could
be better:

- What failed or was missing (the exact tool, args, and error/response).
- Why it got in the way (workaround you had to use, or task you couldn't finish).
- A concrete improvement (new tool, better validation, clearer description, missing
  field, array variant, etc.).

Don't silently work around MCP shortcomings — surfacing them is part of the task.

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
| `synth_sequencer` | `PatternId`, `TrackId`, `NoteId`, `Tick`, `PatternTick`, `Duration`, `Pitch`, `TrackIndex`, `RowIndex`, `SeqInstrumentId`, `NoteGraphId`, `NoteModuleId`                                                          |
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
