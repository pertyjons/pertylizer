# DSP / Audio-Engine Hardening — round 2 (TODO §5 follow-ups)

Branch: `feat/dsp-hardening-2`

A module-wide audit after the first §5 pass found more instances of the **same four
bug classes** in modules the original sweep didn't touch. All sites below were verified
(read line-by-line), not just grep-matched. Each fix mirrors a pattern already landed in
round 1, so the approaches are settled — this is mostly mechanical breadth.

Recommended landing order (cheapest/highest-value → heaviest): **A → B → D → C**.
One commit per class (C may be two). Gate each with the usual
`cargo fmt --check && cargo build && cargo clippy --all-targets && cargo test`.

---

## A — Sanitize the remaining direct CV reads (5.1 class) — *highest value, mechanical*

**Why:** The 5.1 sweep only migrated reads that *already* called `sanitize_cv`. A NaN/Inf
from a direct CV cable still poisons DSP state (silenced/exploded voice). The audit found
**10 unsanitized CV reads in 7 modules**, each feeding a dangerous sink (exp2 / division /
filter coefficients / pan-gain). Two access patterns, same as 5.1:
- `InputReader` (has `.is_connected()`) → raw `reader[i]` is unsanitized → use **`reader.get(i)`**.
- `Option<&AudioBuffer>` (from `inputs.get(Port)`) → `buf[i]` → wrap in **`crate::math::sanitize_cv(buf[i])`**.

### Sites (verified)
| File:line | Port | Receiver kind | Fix | Sink (why it matters) |
|---|---|---|---|---|
| `ring_mod.rs:229` | FREQ_CV | `Option<&AudioBuffer>` (`if let Some(cv) = freq_cv`) | `sanitize_cv(cv[i])` | `apply_cv` → exp2 |
| `fof.rs:676` | vowel_cv | `InputReader` | `.get(i)` | vowel pos → band targets |
| `fof.rs:692` | pitch_cv | `InputReader` | `.get(i)` | semitone → `semitones_to_ratio` (exp2) |
| `fof.rs:696` | breath_cv | `InputReader` | `.get(i)` | breath gate (read unconditionally) |
| `formant_filter.rs:246` | vowel_cv | `InputReader` (`.is_connected()`) | `.get(i)` | IIR bandpass coeffs |
| `vector_mixer.rs:156` | X_CV | `Option<&AudioBuffer>` | `sanitize_cv(cv[i])` | bilinear gains (division) |
| `vector_mixer.rs:159` | Y_CV | `Option<&AudioBuffer>` | `sanitize_cv(cv[i])` | bilinear gains (division) |
| `amplifier.rs:192` | level CV (`cv_in`) | `InputReader` | `.get(i)` | VCA level (× gain) |
| `amplifier.rs:206` | PAN_CV | `InputReader` (`.is_connected()`) | `.get(i)` | pan gain L/R |
| `wavetable_osc.rs:211` | POS_CV | `Option<&AudioBuffer>` (`if let Some(cv) = pos_cv`) | `sanitize_cv(cv[i])` | wavetable position |
| `math_oscillator.rs:606` | mod A | `Option<&AudioBuffer>` (`mod_a.map_or`) | `sanitize_cv(ma[i])` | algo param |
| `math_oscillator.rs:607` | mod B | `Option<&AudioBuffer>` (`mod_b.map_or`) | `sanitize_cv(mb[i])` | algo param |

(12 reads / 7 modules — the `fof` triple and `math_oscillator`/`vector_mixer`/`amplifier`
pairs count as one site each in the "10" headline.)

**Note on `clamp`:** several sites already `.clamp(...)` the value — but `f32::clamp`
returns NaN when the input is NaN, so the clamp does **not** sanitize. The coercion must
happen first.

### Steps
1. Migrate each site per the table. For `InputReader` sites prefer `.get(i)` (the 5.1
   boundary accessor); for `Option<&AudioBuffer>` sites use `sanitize_cv(buf[i])` (the
   existing additive/fractal/math-osc idiom).
2. After the sweep, re-audit: `rg` each module for raw `_cv[i]` / `cv[i]` / modulation-port
   buffer indexing not wrapped or `.get`-read. Confirm only the main audio `IN` port (not a
   CV) reads raw.
3. Test: extend the 5.1 pattern — feed a CV buffer with `f32::NAN`/`INFINITY` into one
   representative newly-fixed module per access pattern (e.g. `formant_filter` vowel CV and
   `ring_mod` FREQ_CV) and assert finite output. The `InputReader::get` boundary test
   already exists; this proves the buffer-based sites too.

### Risk
Low / behaviour-preserving (same coercion, applied at the read). Mechanical.

---

## B — Fix `vocal_tract` stale-rate glottal increment (5.4 class) — *one real bug*

**Why:** `vocal_tract.rs:605` (`note_on`) seeds `self.current_inc = note_freq.as_f32() *
self.inv_sample_rate` using a possibly-stale `inv_sample_rate` (the engine only propagates
the render rate via `ProcessContext` in `process()`). A note triggered before the first
`process()` at a non-default rate seeds the glottal phase increment at the wrong rate →
wrong excitation amplitude / an audible pop on the first block (it then ramps toward the
correct `target_inc` recomputed in `process()`). Same class as the PadSynth bug, milder
(amplitude/click, not a full octave).

### Fix — seed exactly on-pitch in the first `process()` block (per review)
Don't bake the rate in `note_on` *and* don't keep the stale-rate ramp. Use a sentinel so
the first block seeds the increment exactly to the rate-correct target — no glide at all:

1. `note_on`: set `self.current_inc = 0.0;` (a sentinel — a real increment is always
   `freq/sr ≥ 1e-5 > 0`, so 0 unambiguously means "not yet seeded"). Drop the stale
   `note_freq * inv_sample_rate` seed.
2. `process()` (rate already set at the top): compute `target_inc` from the current rate,
   then if uninitialized, snap rather than ramp:
   ```rust
   let target_inc = (self.note_freq.as_f32() * inv_sr).max(1e-5);
   if self.current_inc <= 0.0 {
       self.current_inc = target_inc; // first block: start exactly on-pitch, no glide
   }
   // existing per-sample ramp toward target_inc continues unchanged
   ```
   This preserves the original intent (the comment at `:604` — "start at the note's pitch
   so the first block doesn't glide in") but makes the seed rate-correct. **Verify against
   the actual `process()` ramp code at implementation time** (confirm `current_inc` isn't
   legitimately driven to ≤0 mid-note; it's a `freq/sr` increment so it never is).

### Also: unison onset timing — defer the seconds→samples conversion (per review)
- `voice_synth.rs:722` and `fof.rs:843`: `onset_max = ONSET_MAX_SECS * self.sample_rate`
  is computed in `note_on` (stale rate) → unison onset-stagger off by the rate ratio at
  non-default rates (timing only, not pitch). **Fix:** store the onset delay in *seconds*
  (or as the `(v, note_hz)` inputs) in the voice state at `note_on`, and convert to a
  sample count on the **first `process()` block** using `context.sample_rate`, then trigger
  `voice.restart(...)`. Rate-independent at any rate. (Slightly more restructuring than B's
  core fix — keep it in the same commit but it's the lower-value half.)
- `la_synth.rs` (`update_times` in `note_on`): attack/xfade sample-counts use stale rate,
  but `process()` re-calls `update_times`, so only the first block is off. Lowest priority;
  fix only if trivial.

### Risk
Low. Test: trigger `note_on` at the 48 kHz default, run one `process()` block at 96 kHz,
assert the glottal increment / first-block output is finite and matches the 96 kHz-derived
value (mirror PadSynth's `phase_increment_follows_render_rate_not_note_on_rate`).

---

## C — Move `phase_vocoder` / `spectral_blur` FFT rebuilds off the audio thread (5.3 class) — *heaviest*

**Why:** Both effects rebuild their STFT machinery on the audio thread when `FftSize`
changes (`set_param` is drained on the audio thread):
- `phase_vocoder.rs:365` → `rebuild_stft()` (`:90`) creates 2 `StftProcessor`s + **7
  `vec![…; complex_size]` allocations**.
- `spectral_blur.rs:294` → `rebuild_stft()` (`:78`) creates a `StftProcessor`.

This is the Convolver bug class (FFT planning + heap alloc on the audio thread).

**Severity nuance:** `FftSize` is a **discrete choice parameter** (`FftSizeOption::ALL`,
512–4096), changed rarely by the user — *not* a continuously-automatable knob like the
Convolver's `DecayTrim`. So real-world glitch exposure is low; this argues against the full
worker-thread machinery.

> **Review correction — "rebuild in place" does NOT work.** My first draft proposed
> pre-allocating buffers and reconfiguring the `StftProcessor` in place on `FftSize` change.
> Verified false: `StftProcessor::new` owns a `FftProcessor::new(fft_size)`
> (`spectral.rs:172`), which calls `RealFftPlanner::plan_fft_forward/inverse`
> (`spectral.rs:76–77`) — re-planning **allocates** (planner `Arc`s + twiddle tables). So
> any reconfigure-on-the-audio-thread still allocates. Pre-allocating the *helper Vecs* is
> not enough; the FFT plan itself is the allocation.

### Fix — pre-built STFT pool, switch an index on the audio thread (per review)
Because the FFT size is one of a few discrete `FftSizeOption` values, **pre-build one
`StftProcessor` per option at construction** (off the audio thread, where planning may
allocate), and on `FftSize` change just switch an active index — O(1), zero alloc, zero
re-plan.

1. In `phase_vocoder.rs` / `spectral_blur.rs`, replace the single STFT processor(s) with a
   pool, one entry per `FftSizeOption::ALL` value, built in `new()`:
   ```rust
   // built once, off the audio thread; iterate ALL so the pool tracks the enum
   stft_l_pool: Vec<StftProcessor>, // FftSizeOption::ALL is [Self; 4] (512/1024/2048/4096)
   // new(): FftSizeOption::ALL.iter()
   //          .map(|o| StftProcessor::new(o.size(), o.size() / 4, WindowType::Hann))
   //          .collect()
   // (hop = size/4 — the same convention both effects use inline today; FftSizeOption
   //  has size()/index()/from_index() but no hop() method.)
   ```
   (phase_vocoder has L+R STFTs → two pools; spectral_blur similarly. Index pools by
   `self.fft_size_option.index()`.)
2. Pre-allocate every helper Vec (phase accumulators, magnitude/scratch buffers) to the
   **worst-case** `MAX_BINS = 4096/2 + 1 = 2049` in `new()`, and in `process()` operate on
   the `[..complex_size]` slice for the active size. No `process()`-time `vec!`.
3. `process()`: `let idx = self.fft_size_option.index(); let stft = &mut self.stft_*_pool[idx];`
   and process through the active pool entry.
4. On `FftSize` change in `set_param`: just update `self.fft_size_option`, then `reset()`
   the now-active pool entries and `fill(0.0)` the helper slices — **no allocation, no
   planning, no `rebuild_stft`**. Delete `rebuild_stft`.

Memory overhead is tiny (~tens of KB total across both effects); the win is a 100%
RT-safe, plan-free, O(1) size switch with no worker thread.

Also fold in the **D** private consts here (they live in these two files): replace
`phase_vocoder.rs:20` / `spectral_blur.rs:21` `const MAX_BLOCK_SIZE = 4096` with
`synth_core::MAX_BLOCK_SIZE`.

### Risk
Low–medium (touches the RT path, but no new `synth_dsp` API and no threading). Test:
(a) output for a fixed `FftSize` is unchanged vs. the current code (bit-ish equal, allow
FFT-reorder epsilon); (b) rapid `FftSize` switching interleaved with `process()` stays
finite and never panics; (c) confirm no `process()`-path allocation (pool + pre-sized
buffers). No ear-check strictly required since there's no threading/latency element.

---

## D — Finish the shared-constant unification (5.2 class) — *cleanup, trivial*

**Why:** Six `4096`/`8192` block-size literals escaped the 5.2 sweep and still duplicate
`synth_core::MAX_BLOCK_SIZE`:

| File:line | Literal | Fix |
|---|---|---|
| `synth_engine/src/return_bus.rs:50` | `4096 * 2` (`MAX_FRAME`) | `synth_core::MAX_BLOCK_SIZE * 2` |
| `synth_engine/src/synth_engine.rs:587` | `8192` (audio input ring) | `synth_core::MAX_BLOCK_SIZE * 2` |
| `synth_engine/src/synth_engine.rs:1562` | `4096 * 2` (sidechain buf) | `synth_core::MAX_BLOCK_SIZE * 2` |
| `synth_awe/src/spatial_voice.rs:18` | `SampleCount::new(4096)` | `SampleCount::new(synth_core::MAX_BLOCK_SIZE)` |
| `phase_vocoder.rs:20` | `const MAX_BLOCK_SIZE = 4096` | use `synth_core::MAX_BLOCK_SIZE` (fold into C) |
| `spectral_blur.rs:21` | `const MAX_BLOCK_SIZE = 4096` | use `synth_core::MAX_BLOCK_SIZE` (fold into C) |

Confirm `synth_awe` depends on `synth_core` (it does — uses `SampleCount`). The
192 kHz / `MAX_SUPPORTED` side was already complete after 5.2.

### Risk
Trivial — same values, single source of truth. Gate on workspace build.

### Explicitly NOT in scope (verified legitimate, leave alone)
`padsynth.rs:25` `TABLE_SIZE=4096` (wavetable size), `spectral_blur.rs:17` `MAX_FFT=4096`
(FFT size), `shimmer_reverb.rs:22`, `room_modes.rs:28` `48_000` (1 s @ 48 kHz physics),
oscilloscope/spectrum UI buffers — these are independent values that merely share the
number, not duplications of the engine block ceiling.

---

## Implementation invariants (pre-landing checklist, from review v3)
The v3 review signed off on the plan and flagged three invariants to hold while coding —
two are already specified above, restated here as a single checklist:

1. **Helper Vecs pre-sized to the worst case (C).** In `PhaseVocoder::new` / `SpectralBlur::new`
   every scratch/phase/magnitude Vec (`phase_accum_*`, `mag_*`, etc.) is allocated at
   `MAX_BINS = 4096/2 + 1 = 2049`, and `process()` operates on `[..complex_size]` for the
   active size. No `process()`-path `vec!`/resize-larger.
2. **`current_inc` sentinel is unambiguous (B).** Confirm `current_inc` is a `freq/sr`
   increment (`≥ 1e-5 > 0`) that normal modulation/pitch-glide never drives to `≤ 0`, so
   `current_inc <= 0.0` means *only* "first block after note_on, seed me".
3. **Pool index lookup (C).** `self.fft_size_option.index()` is a small `match → usize` and
   `pool[idx]` is a bounds-checked load — already negligible; no `#[inline]` gymnastics
   needed. (The review suggested annotating it; it's a non-issue, noted for completeness.)

## Sequencing & commits
1. **A** — CV sanitize sweep (1 commit).
2. **B** — `vocal_tract` stale-rate fix (+ opportunistic `voice_synth`/`fof`/`la_synth`
   onset timing) (1 commit).
3. **D** — shared-const cleanup for the engine/AWE sites (1 commit; the two effect consts
   land with C).
4. **C** — phase_vocoder + spectral_blur off-thread/pre-alloc FFT rebuild (1–2 commits;
   heaviest, do last, in-app ear-check ideally).

After each, update this plan's checkboxes. No `plans/TODO.md` entry exists for these
(they're newly-found follow-ups); add a §5 line there only if any class is deferred.
