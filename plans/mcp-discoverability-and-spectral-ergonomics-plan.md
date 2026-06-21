# Plan: MCP discoverability + spectral-tool ergonomics

> **Revision 2 — incorporates a senior audio/synth review.** Refinements:
> (1) `search_modules` = weighted token scoring (name 10 / tags 5 / desc 2 /
> param 2), filter score 0, and a Levenshtein `did_you_mean` instead of
> returning low-relevance matches. (2) `compare_spectra` window selector needs
> tick→seconds translation (via tempo at the tick) so both sources frame
> identically, plus a window-bounds guard (clamp + zero-pad, or a clean
> `OutOfBounds` error). (3) `analyze_sample_spectrogram` must use the file's
> **native sample rate** (never assume the engine's). (4) `math_oscillator`
> docs = a structured `algorithm_parameters` object in `get_module_type_info`.
> Commit order bumps Gap 4 to second (pure-doc, immediate unblock).
>
> **Revision 3 — final review (verdict: ready to implement).** Four refinements:
> (1) clamp the `did_you_mean` Levenshtein threshold (≤2 for short type keys,
> ≤3 for display names) so random strings don't false-match. (2) tick→seconds
> must honour the **full tempo map** (multiple tempo changes in the window) via
> the sequencer's timeline utility, not a single tempo. (3) **CRITICAL** — the
> `log_bins` grid must be **absolute frequency** (e.g. 20 Hz–16 kHz), not
> Nyquist-relative, or comparing a 32 kHz reference against a 44.1 kHz candidate
> compares mismatched physical frequencies and the log-spectral distance is
> invalid. (4) serialise `algorithm_parameters` as an optional
> `#[serde(skip_serializing_if = "Option::is_none")]` field on `ModuleTypeInfo`.

## 0. Where these gaps came from

While timbre-matching a SID voice (Hubbard's Nemesis V2: a triangle ring-modulated
by a fixed 165 Hz phantom oscillator) against Pertylizer patches over MCP, four
concrete gaps surfaced. They are small, real, and each one cost time or quality.
The headline lesson: **gap #1 (discoverability) caused the most damage** — it made
an agent believe Pertylizer had *no ring modulator* and build a worse FM
workaround, when `RingMod` existed all along and reproduces the target's
988±165 Hz sidebands exactly. Fixing discoverability has outsized leverage.

| # | Gap | Symptom that proved it | Priority |
|---|-----|------------------------|----------|
| 1 | `search_modules` misses obvious matches | query `"ring mod multiply"` returned **nothing**, yet `RingMod` exists with name "Ring Mod" + description "Ring modulator — multiplies input with internal carrier oscillator" | **High** |
| 2 | No frame-aligned spectral compare | `compare_spectra` of an alternating tri/noise candidate vs an alternating target both read *unvoiced* → `voicing_mismatch`, partial diff skipped | High |
| 3 | No spectrogram for a sample / WAV | `analyze_spectrogram` is render-only; can't see a reference WAV's per-frame evolution without slicing it externally | Medium |
| 4 | `math_oscillator` params are undocumented | `param_a/b/c` are labelled only "Algorithm parameter A/B/C"; had to probe blindly to learn `param_a` = FM index for `sine_fm` | Medium |

None of these need new DSP. They are search, ergonomics, and documentation.

---

## 1. Gap 1 — `search_modules` must find modules by capability

`search_modules(query)` is the agent's primary "does Pertylizer have an X?"
tool. It currently fails the most basic capability query for a module that
exists. The miss is in `crates/pertylizer/src/mcp_bridge.rs` (the
`search_modules` impl).

**Fix — lightweight weighted token scoring** (no new deps, all in the
`search_modules` impl):
- Split the query on whitespace into tokens; lowercase everything.
- For each module, score each token's presence with field weights:
  **name = 10, tags = 5, description = 2, parameter.name = 2** (sum across
  tokens). Match on prefix/substring so `multiply` hits `multiplies` (cheap
  stemming without a dependency).
- **Sort descending, drop score 0.** `RingMod` (name "Ring Mod" +10, tag
  "ring_mod" +5, desc "...multiplies..." +2) scores well above zero for
  `"ring mod multiply"`.
- **Do NOT pad with low-relevance matches.** Returning unrelated modules when
  nothing really matches misleads an LLM agent into treating the top hit as a
  poor equivalent. Return only score > 0.
- **Empty result → `did_you_mean`.** When nothing scores, compute Levenshtein
  distance of each query token against module names + type keys/prefixes and
  return a `did_you_mean: [...]` field (e.g. `"ringmd"` → `"rng"` / "Ring Mod").
  An empty list with no hint reads as "feature absent" — the exact failure that
  caused the wasted FM detour.
  **Clamp the distance threshold** so a random string doesn't false-match: only
  suggest when distance ≤ 2 against a short type key (`rng`, `mth`) or ≤ 3
  against a display name. `"xyz"` should yield an empty `did_you_mean`, not `rng`.

**Acceptance:** `search_modules("ring mod")`, `search_modules("ring modulator")`,
and `search_modules("multiply two signals")` all return `RingMod` (type key
`rng`) in the top results. Add a regression test
(`crates/pertylizer/tests/`) asserting each of those surfaces `rng`.

> Bonus (cheap, same spirit): a `list_modules_by_category` or a one-line
> "capabilities" tag list in `get_module_type_info` would let an agent enumerate
> "all modulation/ring/FM modules" without guessing query strings.

---

## 2. Gap 2 — frame-aligned spectral comparison

`compare_spectra` (and `analyze_sample_spectrum`) operate over a whole render
or whole sample. For a **time-varying** target — the SID voice switches waveform
every ~20 ms — the only honest comparison is *one voiced frame vs one voiced
frame*. Today there is no way to point either side at a single frame, so an
alternating candidate vs an alternating target both average to "unvoiced" and
`compare_spectra` returns `voicing_mismatch` with an empty partial diff. The
workaround (render a carrier-only, fully-voiced candidate, or pre-slice the WAV
with ffmpeg) works but is friction the tool should remove.

**Fix:** add an optional window selector to each `SpectrumSourceParam` in
`compare_spectra` (and to `analyze_sample_spectrum`):
- `start_ms` (or `start_tick` for renders) + `window_len_ms` → analyse just that
  slice. Reuse the existing STFT framing in
  `crates/pertylizer/src/audio/analysis/spectrum.rs` (the same windowing
  `analyze_spectrogram` already does) — no new DSP, just expose a single-frame
  selection. For samples, slice the decoded buffer before the existing
  `analyze_spectrum` call.
- **Tick→seconds translation (whole tempo map).** A render source addresses
  time in `start_tick`, a sample source in `start_ms`. Convert the render's
  `start_tick` to seconds by **integrating the full tempo map** up to that tick
  (not a single "active tempo" — there may be several tempo changes before/within
  the window). Reuse the **sequencer's existing tick↔time timeline utility** —
  the same one the renderer uses — so both sources are framed identically.
- **Window-bounds guard (no panics).** If the requested `[start, start+len)`
  partially exceeds the render/WAV length, **clamp the start and zero-pad** the
  remainder of the analysis frame. If it is *entirely* out of bounds, return a
  clean `McpBridgeError::OutOfBounds` — never index past the buffer.
- **Phase search is the point.** The SID's 20 ms switching is not aligned to
  engine render boundaries, so exposing `start_ms` lets the agent **slide the
  window in small (~5 ms) steps** to find the exact triangle-frame vs noise-frame
  boundary. Document that usage in the tool description.

**Acceptance:** comparing a single voiced frame of an alternating candidate
against a single voiced frame of an alternating target yields `voicing_mismatch:
false` and a populated `missing_partials` list. Test with a synthetic
tri↔noise alternating render.

---

## 3. Gap 3 — spectrogram for a sample / WAV

`analyze_spectrogram` renders the project and slides an STFT (great — O(1)). But
a reference recording (a real SID render written by `sidplayfp`, any WAV) can
only be analysed as a single aggregate via `analyze_sample_spectrum`; its
**time evolution is invisible**. To compare the per-frame alternation of a
candidate render against the per-frame alternation of the real reference, both
need a spectrogram.

**Fix:** `analyze_sample_spectrogram(sample_id_or_path, hop_ms, window_len_ms,
f0_hint, max_partials, log_bins)` — the exact `analyze_spectrogram` output
(`time_stamps` + per-frame `SpectrumResult`) but sourced from a decoded sample
buffer instead of a render. The STFT loop already exists in
`audio/analysis/spectrum.rs`; this is a second entry point that feeds it the
sample's samples (downmixed to mono at native rate, as `analyze_sample_spectrum`
already does). Pre-allocate the FFT planner/buffers once, same as the render
spectrogram.

> **CRITICAL — use the file's native sample rate.** A reference WAV can be any
> rate (32 kHz raw emulator dump, 48 kHz interface, 44.1 kHz). Read the actual
> rate from the decoded header (`hound` / the sampler provides it) and pass *that*
> to `analyze_spectrum` for the bin→Hz mapping. If the analysis assumes the
> engine's 44.1 kHz on a 32 kHz file, **every** frequency, centroid, and partial
> is shifted and wrong. Do not resample to the engine rate unless explicitly
> asked.

> **CRITICAL (follows from the above) — `log_bins` must be an ABSOLUTE
> frequency grid, not Nyquist-relative.** A `compare_spectra` correctness bug the
> SID case triggers directly. If the target WAV is 32 kHz (Nyquist 16 kHz) and
> the candidate render is 44.1 kHz (Nyquist 22.05 kHz), and the log-bin
> accumulator in `audio/analysis/spectrum.rs` spaces bins relative to *each
> source's* Nyquist, then bin *i* maps to a different physical frequency in each
> source — the log-spectral distance compares e.g. the target's 12 kHz against
> the candidate's 16.5 kHz and is meaningless. **Fix:** define `log_bins` on a
> fixed absolute grid (B bins log-spaced between e.g. 20 Hz and 16 kHz) so bin
> index → identical Hz in both sources. Cap the top at the lower of the two
> Nyquists (or 16 kHz) so no bin sits in a band one source can't represent.
> Applies to the existing `compare_spectra`, not just the new sample tools.

**Acceptance:** `analyze_sample_spectrogram` on a WAV of an alternating
SID voice shows frames flipping `voiced ↔ unvoiced` at the ~20 ms cadence,
matching what `analyze_spectrogram` shows for the equivalent render.

---

## 4. Gap 4 — document `math_oscillator` algorithm parameters

`math_oscillator` (`mth`) has 19 algorithms, each driven by `param_a/b/c`, but
`get_module_type_info` / `get_module_info` describe them only as "Algorithm
parameter A/B/C". To match a target you must know what they do (e.g. for
`sine_fm`, which param is the **ratio** vs the **index** — learned here only by
blind probing, finding `param_a` = index). This is a documentation gap, not a
behaviour change.

**Fix — a structured `algorithm_parameters` table** (static descriptor text,
not live-instance reflection: an agent must know what the knobs do *before* it
builds/edits the patch). In the `math_oscillator` descriptor
(`crates/synth_modules/src/math_oscillator.rs`), add a static per-algorithm
mapping of `param_a/b/c` → `{name, description}`, e.g.:

```json
"sine_fm": {
  "param_a": { "name": "Modulation Index", "description": "FM depth (0–5×)" },
  "param_b": { "name": "Modulation Ratio", "description": "Modulator/carrier ratio (1–8×)" },
  "param_c": { "name": "Phase Offset",     "description": "Modulator start phase" }
}
```

Expose the whole table through `get_module_type_info("math_oscillator")` as a new
structured `algorithm_parameters` field, so a caller reads the complete mapping
in one call. Cover at least the FM/inharmonic algorithms first
(`sine_fm`, `metallic`, `wave_folder`, `feedback_fm`, `phase_dist`), then the
rest. (Live mapped-value reflection in `get_module_info` — "ratio = 2.00" — is a
possible later nice-to-have, but the static table is what unblocks selection.)

**Serialisation:** add the field to `ModuleTypeInfo` as
`#[serde(skip_serializing_if = "Option::is_none")] pub algorithm_parameters:
Option<serde_json::Value>` — backwards-compatible (absent for every module that
doesn't set it) and only emitted for `math_oscillator`.

**Acceptance:** `get_module_type_info("mth")` (or `get_module_info` on an `mth`
instance) tells a caller what `param_a/b/c` mean for the selected algorithm,
at least for the FM/inharmonic algorithms (`sine_fm`, `metallic`, `wave_folder`,
`feedback_fm`).

---

## 5. Out of scope

- **No new DSP / no new ring-mod module.** `RingMod` (`rng`) already exists,
  is registered (`ModuleType::RingMod`, key `rng`), and works over MCP
  (`add_module("ring_mod")` → carrier osc with `carrier_freq`, `carrier_wave`,
  `mix`, `freq_ratio`, `key_track=0` for a fixed modulator). It reproduces the
  SID 988±165 sidebands directly. The earlier "Pertylizer lacks an audio-rate
  ring modulator" claim was **wrong** — caused entirely by gap #1. Nothing to
  build here; just make it findable.
- Spectral-distance *metric* tuning (it favours broadband fullness, so a sparse
  pure-ring spectrum scores worse than a denser FM one even when its primary
  partials are dead-on). Note it in the `compare_spectra` description; don't
  re-engineer the metric in this pass.

---

## 6. Suggested commit sequence

1. **Gap 1 first** (highest leverage, smallest change): `search_modules`
   weighted token scoring across name/tags/description/param, score-0 filter,
   `did_you_mean` (Levenshtein) on empty; regression test that `"ring mod"` /
   `"ring modulator"` / `"multiply two signals"` all surface `rng`.
2. **Gap 4** (pure doc, immediate unblock): static `algorithm_parameters` table
   on `math_oscillator`, exposed via `get_module_type_info`.
3. Gap 2: window selector (`start_ms`/`start_tick` + `window_len_ms`) on
   `compare_spectra` / `analyze_sample_spectrum` sources — reuse the STFT
   framing, add tick→seconds conversion + bounds clamp/zero-pad/`OutOfBounds`;
   frame-aligned test.
4. Gap 3: `analyze_sample_spectrogram` entry point using the file's native
   sample rate; per-frame voiced-flag test.

Each step keeps `cargo fmt --check && cargo build && cargo clippy --all-targets
&& cargo test` green, updates `docs/history.md`, and bumps `Cargo.toml` per the
`new version` flow (per CLAUDE.md).
