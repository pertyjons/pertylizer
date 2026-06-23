# DSP / Audio-Engine Hardening Plan (TODO §5)

Branch: `feat/dsp-hardening`

Covers all four follow-ups deferred from the module critical-review pass. Each is a
**bug-class closure** or **real-time-safety fix**, not a feature. Ordered cheapest →
most architectural; commit one item per logical step (per the `git commit` checklist).

Recommended landing order: **5.2 → 5.4 → 5.1 → 5.3**.
(5.2 is a small/low-risk warm-up; 5.4 is small but fixes a real pitch bug — see below;
5.1 is a mechanical sweep; 5.3 is the heavy architectural one and should go last on its
own.)

---

## 5.2 — Promote block-size & max-sample-rate to shared `pub const`s

> ✅ **DONE** — `synth_core::MAX_BLOCK_SIZE` + `SampleRate::MAX_SUPPORTED` (both the f32
> DSP and u32 config variants) added and re-exported; the three engine `MAX_BUFFER_SIZE`
> consts now alias the shared one, and `compressor`/`limiter` derive from it. Contract
> test `engine_ceilings_are_the_documented_contract` pins the values. Build/clippy/test/fmt
> all green.

**Why:** `MAX_BUFFER_SIZE = 4096` is duplicated **3×** as a private const
(`effect_chain.rs:21`, `voice.rs:49`, `instrument.rs:343`), and two effects hand-copy
engine limits that nobody agrees on:
- `compressor.rs:15` — `MAX_SIDECHAIN_SAMPLES = 4096 * 2` (duplicates `MAX_BUFFER_SIZE`).
- `limiter.rs:22` — `MAX_SAMPLE_RATE = 192_000.0` (no other code agrees; `synth_core`
  tops out at 96 kHz).

If the engine ever raises the block size or supported rate, these copies silently
desync → sidechain buffer truncates a block (wrong gain reduction) or limiter
under-allocates look-ahead (drops the 5 ms guarantee near peaks).

**Approach:** single source of truth in `synth_core`.

### Facts
- DSP `SampleRate` (the f32 one used by `limiter`): `synth_core/src/types/frequency.rs:367`,
  consts `CD_QUALITY/DVD_QUALITY/HIGH_RES` at 377–379. **No MAX const.**
- Config `SampleRate` (u32) + `BufferSize`: `synth_core/src/audio/types.rs:15` /
  `BufferSize` consts at 74–78. **No MAX const.**
- Both `synth_engine` and `synth_modules` already depend on `synth_core`.

### Steps
1. Add `pub const MAX_SUPPORTED: Self = Self(192_000.0);` to the **DSP** `SampleRate`
   in `synth_core/src/types/frequency.rs` (192 kHz — matches the limiter's existing
   assumption; document that this is the engine-wide ceiling and the limiter sizes
   look-ahead from it). Add a `#[must_use]` doc line. **Also** add a matching
   `pub const MAX_SUPPORTED: Self = Self(192_000);` to the **config** `SampleRate(u32)`
   in `audio/types.rs` so the two layers stay in sync (review recommendation — cheap,
   prevents a future desync between the hardware-config and DSP views of the ceiling).
2. Add a shared `pub const MAX_BLOCK_SIZE: usize = 4096;` in `synth_core`. Home it next
   to `BufferSize` in `audio/types.rs` (it is the absolute upper bound on a render block;
   keep the name distinct from the `BufferSize` *newtype* presets). Doc it as "the
   largest block any `process()` will ever be handed; all fixed scratch buffers size
   from this."
3. **Re-export it explicitly.** `synth_core/src/lib.rs` uses an *explicit* re-export list
   (`pub use audio::{ ... BufferSize, ... };` at `lib.rs:29`), **not** a glob — so add
   `MAX_BLOCK_SIZE` to that list or `synth_core::MAX_BLOCK_SIZE` won't resolve.
4. Replace the three private `MAX_BUFFER_SIZE` consts (`effect_chain.rs:21`,
   `voice.rs:49`, `instrument.rs:343`) with `use synth_core::MAX_BLOCK_SIZE;`.
   Update the buffer-alloc sites that referenced the local const.
5. `compressor.rs`: replace `MAX_SIDECHAIN_SAMPLES = 4096 * 2` with
   `MAX_BLOCK_SIZE * 2` (interleaved stereo). Keep the `* 2` local since it is a
   sidechain-specific stereo-interleave factor.
6. `limiter.rs`: replace `MAX_SAMPLE_RATE = 192_000.0` literal with
   `SampleRate::MAX_SUPPORTED.as_f32()`. `MAX_LOOKAHEAD_SAMPLES` then derives from it
   unchanged.

### Risk / test
- Low risk — pure constant unification, same values. Workspace `cargo build`/`clippy`/
  `test` is the gate. Add a tiny `synth_core` unit test asserting
  `MAX_BLOCK_SIZE == 4096` and `SampleRate::MAX_SUPPORTED.as_f32() == 192_000.0` so the
  ceiling is an explicit, documented contract rather than a scattered literal.

---

## 5.4 — PadSynth: fix stale sample-rate (pitch bug — upgraded from low prio)

> ✅ **DONE** — added `current_pitch: Hertz`; `note_on`/`set_voice_pitch` now record the
> pitch, and `process()` derives `phase_increment` from it + the render rate each block
> (clamped via `Hertz::OSC_RANGE`). Regression test
> `phase_increment_follows_render_rate_not_note_on_rate` asserts the increment tracks the
> context rate (96 kHz), not the note_on-time default (48 kHz). Build/clippy/test/fmt green.

**Why (corrected scope — the review caught a real pitch bug my first draft missed):**
PadSynth has **two** sample-rate dependencies, and the original TODO only named one:
1. **Wavetable harmonic-bin placement** — built in `build_wavetable` keyed on
   `self.sample_rate`, lazily in `note_on` (guarded by
   `self.wavetable_dirty || self.sample_rate != self.table_sample_rate`,
   `padsynth.rs:432`). This is the audibly-tiny "one stale note at a boundary" case.
2. **`phase_increment`** — `note_on` computes `self.phase_increment = note_f / sr`
   (`padsynth.rs:441`) using `self.sample_rate`. **This is the real bug.**

The engine propagates the rate to voice-graph modules **only** via `ProcessContext`
inside `process()` — it never calls `set_sample_rate` on them (the `granular_osc`
precedent). PadSynth's struct defaults are `sample_rate: DVD_QUALITY` (48 kHz,
`padsynth.rs:94`) and `base_freq: C4` (`padsynth.rs:82`). So at 96 kHz:
- voice allocated → `sample_rate` still 48 kHz default;
- `note_on` runs **before** the first `process()` → `phase_increment = note_f / 48000`;
- `process()` sets `self.sample_rate = 96000` (`padsynth.rs:324`) but then reads
  `let inc = self.phase_increment;` (`padsynth.rs:339`) — it **never recomputes** the
  increment. With a 48 kHz-derived increment played out at 96 kHz, the table is traversed
  twice as fast → the note sounds **an octave too high**, and stays wrong until the next
  `note_on` or a `set_voice_pitch` call (`padsynth.rs:447`, which also divides by `sr`).

> My first-draft fix ("set `self.sample_rate` at the top of `process()`") is **not
> sufficient** on its own — `process()` captures `phase_increment` as-is, so the stale
> increment survives. The fix must recompute the increment, not just the rate.

### Fix — defer the division (per review)
Stop baking `phase_increment` at `note_on`/`set_voice_pitch`; store the *pitch* and
divide once per block in `process()` where the rate is known to be current.

1. Add `current_pitch: Hertz` to the struct, default `C4` (261.63).
2. `note_on`: replace the `phase_increment = note_f / sr` line with
   `self.current_pitch = note_freq;`. Keep the lazy wavetable rebuild guard as-is
   (the table genuinely depends on the rate; fix #1 below makes the rate current first).
3. `set_voice_pitch`: replace its `phase_increment` recompute with
   `self.current_pitch = freq;`. Apply the clamp at the single recompute site in
   `process()` (step 4), not here, so there is one source of truth.
4. `process()`: after `self.sample_rate = context.sample_rate;` (already at
   `padsynth.rs:324`) and **before** the per-sample loop, recompute once — clamping via
   the shared `Hertz::OSC_RANGE` preset, **not** a raw literal:
   ```rust
   let sr = self.sample_rate.as_f32() as f64;
   let pitch = Hertz::new(Hertz::OSC_RANGE.clamp(self.current_pitch.as_f32()));
   self.phase_increment = f64::from(pitch.as_f32()) / sr;
   let inc = self.phase_increment;
   ```
   `Hertz::OSC_RANGE` (`frequency.rs:70`, `ValueRange::new(1.0, 20_000.0, 440.0)`) is the
   single source of truth for oscillator frequency bounds, and
   `Hertz::new(Hertz::OSC_RANGE.clamp(freq.as_f32()))` is **already the exact idiom used in
   every other voice-pitch module** (`wavetable_osc.rs`, `voice_synth.rs`,
   `granular_osc.rs`, `fof.rs`, … 10+ sites). PadSynth's current hardcoded
   `1.0, 20_000.0` literal is the lone outlier — this both fixes the bug and removes the
   orphaned literal (no future drift; satisfies the CLAUDE.md newtype/preset rule and
   TODO §4.2's drift-lint concern). Numerically identical to today's clamp, so behaviour
   is preserved.

   Now both the table (via the existing `note_on` guard, which sees the correct rate
   because `process()` set it before any subsequent `note_on`) and the increment are
   always consistent with the render rate. This removes the startup glitch with **zero**
   engine plumbing and makes PadSynth behave like the other clean phase-accumulating
   oscillators.

This recompute is one division per block — negligible.

### Risk / test
- Low risk, correctness-positive. Add a `padsynth` unit test: construct at default
  (48 kHz), `note_on(A4)`, then call `process()` with a `ProcessContext` at 96 kHz and
  assert `phase_increment ≈ 440/96000` (not `440/48000`) — i.e. the increment tracks the
  render rate, no octave error. Also keep a table-rebuild assertion (`table_sample_rate`
  follows the render rate across a rate change).

---

## 5.1 — Sanitize CV at the input-read boundary

> ✅ **DONE** — added the sanitizing `InputReader::get(i) -> f32` accessor and migrated all
> 16 `InputReader`-based CV reads (oscillator ×5, filter ×3, lfo, voice_synth ×3,
> vocal_tract ×3, sampler) from `sanitize_cv(reader[i])` to `reader.get(i)`. The 4
> buffer-based osc reads (`inputs.get()` → `Option<&AudioBuffer>`: additive/fractal/math/
> wavetable) and the 4 non-CV scalar `sanitize_cv` calls (signal_monitor, wavetable_data,
> formant_tables, waveshaper) keep the explicit call by design; `math.rs::sanitize_cv`
> stays. Boundary test `input_reader_get_sanitizes_non_finite_cv` added. Build/clippy/test/
> fmt green.

**Why:** NaN/Inf can only enter DSP through a **direct CV-input buffer** (mod-matrix
offsets are already clamped by `ParamModOffsets::effective()` → `norm.clamp(0.0,1.0)`,
`module_traits.rs:1112`). The fix pass added `crate::math::sanitize_cv`
(`math.rs:60`, `x.is_finite() ? x : 0.0`) and wrapped reads **per call-site** — currently
**~24 scattered CV reads** across oscillators/filters/lfo/voice modules. This is
whack-a-mole; the first pass already missed three (`pwm`/`sync`/`pm`). Close the class
by sanitizing **once at the accessor**.

### Facts
- `InputReader` (`module_traits.rs:282`) exposes `is_connected()`, `as_slice()`, and an
  `Index<usize>` impl returning **`&f32`** (`module_traits.rs:301`). A reference-returning
  `Index` cannot sanitize — we need a **value-returning** accessor.
- All current `sanitize_cv` call sites (from the sweep):
  - **CV-input reads to migrate** (`reader[i]` / `cv[i]` style): `oscillator.rs` 549,
    552, 565, 580, 609; `wavetable_osc.rs:203`; `math_oscillator.rs:600`;
    `fractal_osc.rs:291`; `additive_osc.rs:255`; `filter.rs` 412, 417, 725; `lfo.rs:262`;
    `voice_synth.rs` 566, 578, 579; `vocal_tract.rs` 465, 473, 488; `sampler.rs:294`.
  - **NOT InputReader reads — leave as `sanitize_cv`:** `wavetable_data.rs:44`
    (`position`), `formant_tables.rs:71` (`vowel`), `signal_monitor.rs:265` (`sample`),
    `effects/waveshaper.rs:69` (`driven`), and the internal uses in `math.rs` `hard_clip`
    / `foldback`. These don't read a CV buffer directly; keep them.

### Approach
Add a sanitizing value accessor to `InputReader` and migrate the CV-buffer reads to it.

### Steps
1. Add to `InputReader` (`module_traits.rs`):
   ```rust
   /// Read sample `i`, coercing non-finite (NaN/Inf) to 0.0.
   /// This is the single sanitize boundary for direct CV-input cables —
   /// mod-matrix offsets are already clamped in `ParamModOffsets::effective`.
   #[inline]
   #[must_use]
   pub fn get(&self, i: usize) -> f32 {
       let v = match self.buffer { Some(b) => b[i], None => self.default };
       if v.is_finite() { v } else { 0.0 }
   }
   ```
   (`self.default` is already finite, so the branch only ever fires for live buffers.)
2. Migrate each CV-input read in the list above: `crate::math::sanitize_cv(reader[i])`
   → `reader.get(i)`. Where a module reads via `as_slice()` + manual indexing instead of
   the `Index` impl, either switch to `get(i)` or keep a slice-level sanitize — confirm
   per site during the sweep (most use `reader[i]`).
3. After migration, verify no CV-buffer read still uses the raw `Index` (`reader[i]`)
   without sanitize: `rg '\w+_(reader|cv)\[i\]'`-style audit across the module list. The
   `Index` impl can stay for non-CV/other consumers, but document that **CV inputs must
   use `get`**.
4. Leave the 4 non-CV `sanitize_cv` sites + the `math.rs` internal uses untouched.
   Do **not** remove `sanitize_cv` from `math.rs` (still used by waveshaper/foldback/etc.).

### Risk / test
- Medium-mechanical. Behaviour-preserving (same coercion, moved earlier). Gate on
  workspace build/clippy/test. Add a focused unit test: feed a CV buffer containing
  `f32::NAN` / `INFINITY` into one representative module (e.g. `filter` cutoff CV) via a
  `process()` call and assert the output is finite. Document the boundary contract in the
  `InputReader` doc comment so future CV reads use `get`.

---

## 5.3 — Convolver: build the IR off the audio thread (heaviest; do last)

> ✅ **DONE** — implemented the per-instance worker-thread design. synth_dsp gained
> `IrSpectra` (opaque spectra pool), `IrSpectraBuilder` (off-thread FFT builder), and
> `PartitionedConvolver::{reserve_partitions, swap_ir_spectra}` (alloc-free pointer swap).
> `Convolver` spawns a parked `convolver-ir` worker in `new()`; `set_param(Ir/DecayTrim)` /
> `set_sample_rate` enqueue a request + unpark; `process()` drains a finished build and
> swaps it in lock-free, sending the old spectra back for off-thread drop. `Drop` signals +
> unparks + joins. Pools pre-reserved to `MAX_PARTITIONS` (188) so swaps never allocate.
>
> **Adversarial review fix:** the worker now **holds an undelivered result and retries**
> rather than dropping it when the result slot is full — the original drop-newest-on-full
> could strand the convolver on a stale IR (no convergence to the latest). Tests:
> `swap_ir_spectra_matches_inline_update_ir` (bit-exact equivalence with the old path),
> `convolver_spawns_and_drop_joins_worker` (no thread leak), `off_thread_rebuild_delivers_…`
> (async delivery + finite output), `rapid_ir_changes_settle_without_livelock` (convergence).
> Known accepted residual: `unpark()` is a bounded non-blocking futex wake on the audio
> thread, but only on IR-change events — steady-state `process()` does one lock-free
> `try_pop` and nothing else. Build/clippy/test/fmt green. **Not yet ear-verified in-app**
> (the plan's manual "automate DecayTrim, listen for dropouts" check still wants doing).

**Why:** `Convolver::rebuild_ir` (`convolver.rs:228`) is invoked from `set_param`
(`convolver.rs:491` for `Ir`, `505` for `DecayTrim` with a `>0.01` delta guard), which
drains **on the audio thread**. It regenerates three IR variants and calls
`update_ir` on **six** `PartitionedConvolver`s (medium L/R + soft L/R + loud L/R), each
running up to `MAX_IR_SAMPLES/PARTITION_SIZE` = 96000/512 ≈ **188 forward FFTs**. Rapid
`DecayTrim` automation re-partitions all six IRs every move → sustained CPU spike /
dropout risk. Steady-state is allocation-free (`update_ir` reuses pools,
`spectral.rs:393`), but the FFT work itself is unbounded-for-the-hot-path.

There is also a residual **first-growth allocation** in `ensure_partition_buffers`
(`spectral.rs:430`) that is only safe because `new()` seeds the longest default IR
(Plate); a shorter default would reintroduce an audio-thread alloc.

**Approach (revised per review): a per-instance background worker thread.** The first
draft proposed an engine-level rebuild worker reachable through `ProcessContext` — that
is a whole new engine subsystem. The review pointed out a much simpler, fully
self-contained alternative that the facts support:

- `Convolver` is a long-lived **`AudioEffect`** (`convolver.rs:361`), instantiated once
  or twice per channel at an insert point — **not** a per-voice `PolyModule` that gets
  cloned across a voice cluster. So there are only a handful of instances, and the cost
  of one parked worker thread each is negligible.
- The `AudioEffect` trait carries **no `Clone` bound** (Convolver impls only `Default`,
  `Describable`, `AudioEffect`), so the struct can own a `JoinHandle` + ringbuf endpoints
  without fighting a derive.

So: **spawn a dedicated worker thread inside `Convolver::new()`**, keep the whole
mechanism in `convolver.rs`, and touch nothing in `ProcessContext`/`module_traits.rs`/the
engine.

### Design
The handoff unit is the **precomputed partition spectra** (`Vec<Vec<Complex<f32>>>`),
not the whole `PartitionedConvolver` (which owns FFT planners + running state we must not
move across threads).

1. **Extract IR-spectra computation as a pure function:** given `(ir_type, sample_rate,
   decay_trim, partition_size, fft_size)`, produce the **six** variants' partition-spectra
   pools (medium L/R, soft L/R, loud L/R — though L and R share an IR, so really three
   distinct spectra sets the audio thread fans out to six convolvers). This is the
   expensive part (`fill_ir*` + the forward FFTs currently inside `update_ir`). No
   `&mut self`, no engine state → runnable off-thread.
2. **Worker thread (in `new()`):** `ringbuf` is 0.5.0 (SPSC, lock-free, non-blocking
   pop). Spawn the worker and **park** it when there is no work — a busy `pop()` spin
   would burn a full core:
   ```rust
   let (req_tx, req_rx) = ringbuf::HeapRb::new(1).split();   // cap-1 — see coalescing
   let (res_tx, res_rx) = ringbuf::HeapRb::new(2).split();
   let worker = std::thread::spawn(move || worker_loop(req_rx, res_tx));
   ```
   Store `req_tx`, `res_rx`, and the `JoinHandle` as **`Option<…>`** (needed for the Drop
   handshake below). Worker loop:
   ```rust
   fn worker_loop(mut req_rx, mut res_tx) {
       loop {
           if let Some(req) = req_rx.pop() {
               // fill_ir* + forward FFTs → build the spectra pools, push to res_tx
           } else if req_rx.is_closed() {
               break;                 // producer dropped on Drop → exit so join() returns
           } else {
               std::thread::park();   // sleep until unparked
           }
       }
   }
   ```
   > **Gotcha (must get right):** the park branch must be guarded by an `is_closed()`
   > check *first*, otherwise the Drop handshake hangs — after `Drop` drops `req_tx` and
   > unparks, `pop()` returns `None`, and a naive "pop-else-park" loop would simply park
   > again forever, so `join()` blocks. `ringbuf` 0.5 exposes producer-dropped via the
   > consumer's `is_closed()`; an explicit shutdown sentinel works too.
3. **Request on `set_param(Ir|DecayTrim)`:** instead of calling `rebuild_ir` inline, push
   a small request payload `(ir_type, decay_trim)` to `req_tx` and `worker.unpark()`.
   Keep the existing `Ir != current` and `DecayTrim` `>0.01` delta guards so we don't
   enqueue no-op rebuilds. (`set_param` runs on the audio thread; `push` + `unpark` are
   both cheap and lock-free.)
4. **Drain + swap on the audio thread** (top of `process()`): non-blocking `pop` on
   `res_rx`. If a result is present, `std::mem::swap` each convolver's `ir_spectra` pool
   with the freshly-built one and reset its running accumulators — **no FFT, no alloc**
   (the FFTs already ran on the worker). Add a
   `PartitionedConvolver::swap_ir_spectra(&mut self, new_pool)` for exactly this.
5. **Off-thread drop:** the *old* pools that `mem::swap` displaced must not be `Vec`-
   dropped on the audio thread. Send them back to the worker over a third queue (or
   reuse the request channel's payload to carry "here are the buffers to reuse/drop"),
   mirroring the engine's `script_trash` deferral pattern (`synth_engine.rs:541`).
   Bonus: returned buffers can be **recycled** as the next build's scratch, avoiding
   re-allocation entirely.
6. **Coalescing — capacity-1 queue + dirty flag (review pattern).** Rapid `DecayTrim`
   automation must not flood or block the worker. Size `req_tx` to **1** and add a
   `rebuild_dirty: bool` to `Convolver`:
   - On param change: if `req_tx.push(payload).is_ok()` → unpark; if it returns `Full`
     (worker still building) → set `self.rebuild_dirty = true` (drop the redundant
     request — the latest value is captured by the flag).
   - In `process()`, right after popping a finished result: if `self.rebuild_dirty`, push
     a fresh request built from the **current** `(ir_type, decay_trim)`, unpark, and clear
     the flag.

   This guarantees the worker is never flooded, never blocks the audio thread, allocates
   nothing, and always catches up to the latest parameter value the instant it finishes
   its current FFT run. Until a rebuild lands the current IR keeps playing — graceful,
   just slightly stale mid-sweep.
7. **Kill the first-growth alloc for good:** pre-size every `PartitionedConvolver`'s
   `ir_spectra` *and* `input_spectra` pools to the absolute max
   `MAX_IR_SAMPLES / PARTITION_SIZE = 96000 / 512 = 188` partitions at construction
   (in `new()`, off the audio thread). Then `ensure_partition_buffers` can never allocate
   on a later longer IR — closes the residual hazard the review flagged.

### Lifecycle caveat (must handle)
Spawning a thread in `new()` means **`Convolver` needs a `Drop` impl** that signals the
worker to exit and `join`s it — otherwise each dropped Convolver leaks a parked thread
(e.g. on project reload / instrument delete). Hold the endpoints + handle as `Option`
and run the handshake in `Drop`:
```rust
impl Drop for Convolver {
    fn drop(&mut self) {
        self.req_tx = None;                 // drop producer → consumer.is_closed() == true
        if let Some(h) = self.worker_handle.take() {
            h.thread().unpark();            // wake it so it observes the close (step 2 loop)
            let _ = h.join();
        }
    }
}
```
The `unpark()` before `join()` is what makes the step-2 `is_closed()` branch reachable —
without it a parked worker never wakes to see the dropped producer and `join()` hangs.
Confirm no construction path spawns Convolvers in a hot loop (it shouldn't — effects are
built at graph-build time), and that offline render constructs at most a handful.

### Latency contract
A few-ms-late IR swap during automation is acceptable and far better than a dropout —
document this in the worker's doc comment.

### Risk / test
- **Highest risk** of the four — real-time threading — but the per-instance design keeps
  the blast radius inside `convolver.rs`. Land it isolated, after the other three. Tests:
  (a) a unit test that the extracted pure spectra-builder produces bit-identical output to
  the current inline path for each IR type; (b) a stress test rapidly setting `DecayTrim`
  and asserting no panic / output stays finite; (c) confirm steady-state `process()` does
  zero allocation (existing RT discipline); (d) a `Drop`/lifecycle test that constructing
  and dropping a Convolver joins its worker thread (no leak). Manual: load a Convolver,
  automate `DecayTrim` fast, listen for dropouts before/after.

---

## Sequencing & commits
1. `5.2` — shared consts (1 commit). Warm-up, touches several files but mechanical.
2. `5.4` — PadSynth defer-the-division pitch fix (1 commit).
3. `5.1` — `InputReader::get` + CV-read sweep (1 commit, possibly 2: accessor, then sweep).
4. `5.3` — Convolver per-instance off-thread IR build (its own series of commits).

Update `plans/TODO.md` checkboxes as each lands. After 5.3, if a second heavy module ever
wants the same off-thread-rebuild treatment, revisit whether to generalize the
per-instance worker into a shared engine-level one (note it in the worker's doc comment).
