# Plan: `OfflineNoteSession` — engine reuse across patch-sweep steps

> From TODO §5.2 (third bullet). **Performance**, not a bug — the sweeps are
> correct today, just slow. No user-visible behavior change intended.

## 0. Why

`analyze_instrument_range_impl` and `analyze_velocity_response_impl`
(`crates/pertylizer/src/mcp_bridge.rs`) render one note per swept value through
`analyze_rendered_note` → `audio::preview::render_note_to_buffer`
(`crates/pertylizer/src/audio/preview.rs`). **Each call spins up a fresh
`SynthEngine::new()` and reloads the instrument's module graph + sample data.**

- `analyze_instrument_range` default sweep = one note per octave over a range →
  up to ~10 renders; a 1-semitone step over the full MIDI range = 60+ fresh engines.
- `analyze_velocity_response` default = 8 velocity steps = 8 fresh engines.

The per-render engine build + graph/sample reload dominates wall-clock for these
tools. The fix mirrors the existing `OfflineEngineSession`
(`crates/pertylizer/src/audio/arrangement_render.rs:219`, ctor `new`/`new_with_scope`,
`render_range` reuses one engine across calls): build the engine + load the patch +
samples **once**, then render each swept note against that reused engine.

## 1. The component — `OfflineNoteSession`

New struct in `crates/pertylizer/src/audio/preview.rs` (next to
`render_note_to_buffer`, sharing its per-note render internals):

```rust
pub struct OfflineNoteSession {
    engine: SynthEngine,
    // + the loaded InstrumentId, sample rate / block context, scratch buffers,
    //   and whatever `render_note_to_buffer` currently builds per call.
}

impl OfflineNoteSession {
    /// Build the engine, load the instrument graph + sample data once.
    pub fn new(
        session: &SynthSession,
        sample_library: &SharedSampleLibrary,
        instrument_id: InstrumentId,
    ) -> Result<Self, …> { … }

    /// Render one note against the reused engine.
    pub fn render(
        &mut self,
        note: MidiNote,
        velocity: Velocity,
        duration_ms: Milliseconds,
        tail_ms: Milliseconds,
    ) -> RenderedNote { … }
}
```

### The hard part — voice bleed between renders
`OfflineEngineSession` already solves this: consecutive renders on one engine must
not let a previous note's release tail/voice state leak into the next render. Two
options, pick one and document it:
1. **Drain to silence** between renders (process blocks with all-notes-off until
   the voice pool is idle / output below a floor), like `OfflineEngineSession`'s
   inter-range handling — preferred, it reuses the proven approach.
2. **Reset voice state** explicitly (note-off-all + envelope reset) if cheaper.

This is the correctness-sensitive bit: the whole point is bit-identical output to
the current one-engine-per-note path, so the drain policy must guarantee no
cross-note contamination.

## 2. Refactor the call sites

In `mcp_bridge.rs`:
- `analyze_instrument_range_impl`: build one `OfflineNoteSession`, loop the
  `sweep_range` calling `session.render(note, …)` instead of
  `analyze_rendered_note` (which itself wraps `render_note_to_buffer`). Keep the
  per-note metric extraction identical.
- `analyze_velocity_response_impl`: same, sweeping velocity.

`analyze_rendered_note` / `render_note_to_buffer` stay for any single-shot caller
(`analyze_note`, `preview_note`); `OfflineNoteSession::render` factors out the
shared per-note body so there's one implementation, not two.

## 3. Tests

- **Determinism / equivalence (the key guard).** Mirror
  `tests/arrangement_render_determinism.rs::session_render_range_is_bit_exact_across_three_calls`:
  assert that rendering the same note three times through one `OfflineNoteSession`
  is **bit-exact**, and that a sweep via the session is **bit-exact** to the old
  per-note `render_note_to_buffer` path (proves no behavior change + no voice bleed).
- Keep the existing `analyze_instrument_range` / `analyze_velocity_response` tests
  green (they already assert the metric outputs).

## 4. Optional follow-on (separate commit)

After session-reuse lands and is proven bit-exact, parallelize the sweep target
vector with `rayon::par_iter` for a further 2–4× — but only if each render is
fully independent (it is, once each gets its own session or the session is
`Send` + per-thread). Note: parallelism + a single reused engine conflict, so
par_iter would need **one session per worker**, not one shared session. Decide at
that point; the sequential reuse is the bulk of the win.

## 5. Risks / notes

- **RT-safety doesn't apply** — this is the offline (non-audio) thread, like
  `OfflineEngineSession`.
- The only real risk is the voice-bleed drain getting the equivalence subtly
  wrong; the bit-exact test is the gate.
- Scope: do **not** change `analyze_note`/`preview_note` output; only speed up the
  multi-step sweeps.

## 6. Suggested commit sequence

1. `OfflineNoteSession` (struct + ctor + `render`, factoring the shared body out of
   `render_note_to_buffer`) + the bit-exact determinism test.
2. Switch `analyze_instrument_range_impl` / `analyze_velocity_response_impl` to the
   session; confirm existing tests still pass.
3. (Optional, later) per-worker parallel sweep.
