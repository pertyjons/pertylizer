# OfflineNoteSession — engine reuse across patch-sweep steps

Implements TODO §6.2 item 3. Amortizes the offline-engine + patch-load setup
across the per-step renders done by the two sweep tools.

## Problem

`render_note_to_buffer` (`crates/pertylizer/src/audio/preview.rs:138`) does three
things per call:

- **(a)** snapshot the live instrument + build a fresh `SynthEngine`, load its
  module graph / effects / connections / sample data
- **(b)** warm-up process one block
- **(c)** play one note and capture the buffer

`analyze_instrument_range_impl` / `analyze_velocity_response_impl`
(`crates/pertylizer/src/mcp_bridge.rs:7366` / `:7426`) call it once per swept
value via the `sweep_range` loop, so they pay (a)+(b) on every step — 60 fresh
engines for a default semitone sweep, 8 for the default velocity sweep.

## Fix

Mirror the proven `OfflineEngineSession` (`audio/arrangement_render.rs:219`): do
(a) once at construction, make (b)+(c) a per-call method with a voice-bleed drain
between renders.

### New `OfflineNoteSession` (in `preview.rs`)

```rust
pub struct OfflineNoteSession {
    engine: SynthEngine,
    handle: EngineHandle,
    octave_offset: i32,    // captured once, applied to each note
    channels: u16,
    first_call_done: bool, // gates warm-up vs. drain, like OfflineEngineSession
}
```

- `new(session, sample_library, instrument_id) -> Result<(Self, Vec<String>), McpBridgeError>`
  — everything in `render_note_to_buffer:146..419` (validate, snapshot, build
  engine, load patch, enable instrument, mirror volume/pan/solo, `on_stream_start`)
  **minus** the warm-up. Returns setup warnings separately, like
  `OfflineEngineSession::new`.
- `render(note, velocity, duration_ms, tail_ms) -> Result<RenderedNote, _>`:
  1. `fastrand::seed(OFFLINE_RENDER_SEED)` (per render, matching `render_range:395`)
  2. compute `effective_note` from `octave_offset`
  3. frame counts + `total_frames == 0` guard
  4. if `first_call_done` → voice-bleed drain (from `render_range:476..488`);
     else → warm-up block + set `first_call_done = true` (`render_range:543..547`)
  5. NoteOn → render loop → NoteOff → return `RenderedNote` (`preview.rs:451..523`)

Ordering reproduces today's single-render output bit-for-bit: first call is
`seed → warmup → render` (current sequence); later calls `seed → drain → render`
(the fresh-state pattern `OfflineEngineSession` already validates).

### `render_note_to_buffer` becomes a thin wrapper

Keeps all 6 existing call-sites (preview_integration.rs, satb_patches.rs,
gui/analyze.rs, the `preview_note` tool) untouched:

```rust
let (mut sess, setup_warnings) = OfflineNoteSession::new(session, sample_library, instrument_id)?;
let mut rendered = sess.render(note, velocity, duration_ms, tail_ms)?;
rendered.warnings = [setup_warnings, rendered.warnings].concat();
Ok(rendered)
```

### Wire the two sweeps (`mcp_bridge.rs`)

Add a session sibling to `analyze_rendered_note` (keep the original — the
single-note `analyze_note` tool still uses it):

```rust
fn analyze_rendered_note_in_session(sess: &mut OfflineNoteSession, note, velocity,
    duration_ms, tail_ms, expected_note) -> Result<AnalyzeNoteResult, _>
```

In each `*_impl`, build the session once, fold its setup warnings into
`warnings`, then have the `sweep_range` closure (`FnMut`) capture `&mut sess` and
call the session variant.

## Tests (mirror `arrangement_render_determinism.rs`)

1. **Equivalence** (key gate): notes N1/N2/N3 through one `OfflineNoteSession`
   are bit-identical to three independent `render_note_to_buffer` calls — proves
   reuse doesn't alter output. Use a simple (drain-converging) patch.
2. **Determinism**: same note rendered 3× on one session is bit-exact.

## Scope / risk

- Files: `preview.rs` (extract + new type), `mcp_bridge.rs` (2 sweep fns + 1
  helper), 1 test file.
- Low risk — pure extract-and-amortize; behavior preserved by the wrapper. Same
  drain caveat as arrangement renders (long reverb/delay tails best-effort across
  renders); analysis metrics already tolerate it.

## Deferred follow-up

Parallelizing the sweep (TODO's "par_iter" line) needs one session per rayon
thread (the engine isn't shared across threads). Separate step, after this lands.
