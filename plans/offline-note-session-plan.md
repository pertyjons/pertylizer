# Plan: `OfflineNoteSession` — engine reuse across patch-sweep steps

> **STATUS — IMPLEMENTED** in commit `c2a3226` ("Add OfflineNoteSession to
> amortize engine setup across patch sweeps"). The component, the call-site
> refactor, and the bit-exact equivalence test all shipped as designed below.
> This doc now records what shipped + the review-derived follow-up hardening
> that is **not** yet done.
>
> From TODO §5.2. **Performance**, not a bug — the sweeps were correct before,
> just slow. No user-visible behavior change intended (and the bit-exact test
> proves it for dry patches — see the long-tail caveat in §4).

## 0. Why

`analyze_instrument_range_impl` and `analyze_velocity_response_impl`
(`crates/pertylizer/src/mcp_bridge.rs`) rendered one note per swept value through
`render_note_to_buffer`, each call spinning up a fresh `SynthEngine::new()` and
reloading the instrument's module graph + sample data (60+ fresh engines for a
1-semitone full-range sweep; 8 for the default velocity sweep). The engine
build + graph/sample reload dominated wall-clock.

The fix mirrors `OfflineEngineSession`
(`crates/pertylizer/src/audio/arrangement_render.rs`): build the engine + load the
patch + samples **once**, render each swept note against the reused engine.

## 1. What shipped — `OfflineNoteSession`

`crates/pertylizer/src/audio/preview.rs`:

```rust
pub struct OfflineNoteSession {
    engine: SynthEngine,
    handle: synth_engine::EngineHandle,
    octave_offset: i32,     // patch octave shift, captured once
    first_call_done: bool,  // gates warm-up (first call) vs. drain (later calls)
}

impl OfflineNoteSession {
    pub fn new(session, sample_library, instrument_id)
        -> Result<(Self, Vec<String>), McpBridgeError>;   // (session, setup warnings)
    pub fn render(&mut self, note, velocity, duration_ms, tail_ms)
        -> Result<RenderedNote, McpBridgeError>;
}
```

- **Setup once** (`new`): snapshot the live instrument, build a fresh engine, load
  graph / effects / connections / sample data. Returns non-fatal setup warnings.
- **Per note** (`render`): first call warms up (one process block so queued
  commands apply); subsequent calls run the **voice-bleed drain** first, then
  play the note. `fastrand` is re-seeded with `OFFLINE_RENDER_SEED` each render
  for determinism. Block sizes are trimmed for sample-accurate note-off
  (`this_buffer = remaining.min(BUFFER_SIZE).min(block_cap)`).
- A thin `render_note_to_buffer` wrapper builds a one-shot session for the
  single-render callers (`analyze_note`, `preview_note`) — one implementation.

### Voice bleed between renders (the design decision)
Chosen approach: **drain to silence** with a cap, mirroring `OfflineEngineSession`
exactly (`VOICE_DRAIN_MAX_MS = 400.0`, `DRAIN_SILENCE_EPSILON = 1e-7`). Before each
non-first render the engine processes silent blocks until output falls below the
floor or the cap is hit. This is the same accepted tradeoff the arrangement
renderer already makes for soloed-track tails — see §4.A for its one real limit.

## 2. Call-site refactor (shipped)

`mcp_bridge.rs`: `analyze_instrument_range_impl` and
`analyze_velocity_response_impl` build one `OfflineNoteSession` and loop the sweep
calling `sess.render(...)` instead of a fresh engine per step; per-note metric
extraction is unchanged. `analyze_note` / `preview_note` keep the one-shot path.

## 3. Tests (shipped)

`tests/preview_integration.rs::session_render_matches_independent_renders_bit_exact`
renders four notes through a reused session and asserts **bit-exact** equality
with the same notes rendered through independent fresh-engine
`render_note_to_buffer` calls — the correctness gate proving the drain resets
state between renders. **Coverage caveat:** it uses `sustain_patch_no_envelope()`
(dry, no tail), so it does not exercise the long-tail case in §4.A.

---

## 4. Follow-up hardening (from senior DSP review, 2026-06-22 — NOT yet done)

Verified against the shipped code; severity calibrated.

### A. Long effect/envelope tails defeat the 400 ms drain — ✅ DONE (commit `9c1fec4`)
**Resolved:** `EngineCommand::ResetDsp` now hard-resets all per-instrument signal-path
DSP (voices + effect chain + oversampling downsamplers) plus the master/return
effect chains and the modular graph instantly. `OfflineNoteSession::render` sends
it before each non-first render (applied by the warm-up block) instead of the old
drain, giving bit-exact isolation regardless of tail length. New guard
`session_render_wet_patch_is_tail_proof_bit_exact` (8 s reverb) passes — it could
not under the 400 ms drain. **Not** reset (documented, out of the offline path):
the AWE room simulation and the one-block sidechain previous-output buffer. The
original analysis is kept below for context.


The drain caps at `VOICE_DRAIN_MAX_MS = 400.0` and the code comment already says
*"Long reverb/delay tails are best-effort."* A patch with a multi-second reverb /
delay-feedback tail will **not** reach `DRAIN_SILENCE_EPSILON` within 400 ms, so
note *N*'s tail bleeds into note *N+1*'s render → not bit-exact, and skews the
spectral metrics for later sweep steps on wet patches. This is the **same**
limitation `OfflineEngineSession` accepts for soloed-track tails, so it's a
consistent, known tradeoff — not an oversight — but worth closing for wet patches.

- **The clean fix needs new engine support.** The only reset command today is
  `EngineCommand::AllNotesOff` (`commands.rs:404`), which flips voices into
  *release* — it does **not** zero delay lines / reverb buffers / biquad state. A
  true instant reset would be a new `EngineCommand::ResetDsp` (a.k.a. `Panic`) that
  zeroes voice + effect DSP state, giving 0 ms, 100% bit-exact isolation with no
  drain-block CPU. This belongs in `synth_engine` and would also let
  `OfflineEngineSession` drop its drain. Medium effort (touch every stateful
  module's reset); design separately.
- **Cheaper interim:** raise/uncap the drain when a patch has effects, or expose
  the residual energy as a render warning so callers know a step may be
  contaminated. Lower value than the real reset.
- **Test gap to close with it:** add a bit-exact case with a reverb/delay patch
  (currently only the dry `sustain_patch_no_envelope` is covered).

### B. Per-render scratch allocation — trivial, optional
`render` allocates `let mut block = vec![0.0; BUFFER_SIZE * CHANNELS]` every call
(preview.rs ~476). On the offline thread this is harmless (RT rules don't apply),
but it's free to cache `block` as an `OfflineNoteSession` field allocated in
`new()`. The output `samples` `Vec` must stay per-render (it's the returned
buffer); a render-into-caller-buffer variant is possible but not worth it. Low
priority.

### C. Variable final-block size — already a codebase-wide assumption
The sample-accurate note-off trimming yields small final blocks (down to 1
sample), which changes the control-rate cadence at that boundary. This is the
**same** thing `OfflineEngineSession` already does, so every `synth_modules` /
`synth_dsp` module is already required to be block-size-agnostic and the live
renders depend on it. Action is *verify/document* that invariant, not *fix* —
lower severity than a new risk. If a module ever proves block-size-sensitive, the
alternative is a constant `BUFFER_SIZE` with a sub-block note-off offset handled
inside the voice, but there's no evidence that's needed.

### D. Parallel sweep needs a step-count threshold — refinement of the §5 note
`render` takes `&mut self` over one engine, so `rayon::par_iter` needs **one
session per worker** — i.e. *N* engine builds + sample loads. For small sweeps
(8 velocity steps) that overhead can exceed the rendering it parallelizes; only
worth it for large sweeps where the build cost amortizes. So: gate parallel
multi-session sweeps behind `steps > worker_count * threshold`, where `threshold`
≈ engine-build-cost / per-render-cost. Sequential reuse (shipped) is already the
bulk of the win; parallelism is a conditional extra.

## 5. Status checklist

| Item | Status |
| :--- | :--- |
| Engine + sample amortization (`OfflineNoteSession`) | ✅ shipped (`c2a3226`) |
| Call-site refactor (range + velocity sweeps) | ✅ shipped |
| Bit-exact equivalence test (dry patch) | ✅ shipped |
| §4.A long-tail isolation (`EngineCommand::ResetDsp`) | ✅ shipped (`9c1fec4`) |
| §4.A wet-patch bit-exact test (8 s reverb) | ✅ shipped (`9c1fec4`) |
| §4.B cache scratch `block` | ☐ optional micro-opt |
| §4.C confirm/doc block-size-agnostic DSP | ☐ verify only |
| §4.D parallel sweep + step-count threshold | ☐ optional, large sweeps only |
