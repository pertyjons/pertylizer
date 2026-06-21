# Plan: spectral-analysis MCP tools (timbre-matching loop)

> **Revision 2 — incorporates a senior DSP review.** Six corrections folded in:
> (1) **Newtype pattern enforced** — `Hertz`/`Decibels`/`Cents`/`Seconds`/
> `NormalizedValue` from `synth_core::types`, never raw `f32`, per GEMINI.md.
> (2) **Short-window precision** — mandatory zero-pad to a power of two +
> selectable window (Blackman-Harris default) + parabolic peak interpolation.
> (3) **STFT is now core, not deferred** — `analyze_spectrum` renders **once**
> and slides the window in Rust (O(1) MCP calls), never N per-window renders.
> (4) **Robust, fail-safe f0** — normalized ACF / YIN with an *unvoiced* verdict
> (`f0 = None`) so SID noise frames don't poison the harmonic metrics.
> (5) **Reuse `hound`** via the existing `audio/export.rs` writer — no hand-rolled
> WAV header. (6) **Edge cases** — silence/DC and Nyquist guards.
>
> **Revision 3 — second DSP review (verdict: production-ready).** Five
> implementation refinements folded in: (a) parabolic-interpolation denominator
> guard; (b) `f0_hint` restricts the ACF/YIN **lag-search range** (not a bypass),
> killing octave errors while still tracking vibrato/detune; (c) explicit
> **unvoiced comparison** rules in `compare_spectra`; (d) **pre-allocate** the
> FFT planner + buffers once and reuse across STFT frames; (e) RMS-envelope
> time-alignment note for the agent.

## 0. Why this exists — the problem to solve

An external agent (driving Pertylizer over MCP) is trying to **reconstruct a
target timbre** — concretely, reproduce a Commodore-64 SID voice (Rob Hubbard's
Nemesis, voice 2: a triangle ring-modulated by a silent phantom oscillator,
alternating with chip-noise every video frame) as a Pertylizer patch, and to do
it **mathematically** rather than by ear-guessing parameter values.

Today that agent is effectively blind. The only spectral feedback Pertylizer
exposes over MCP is `analyze_mix_bus`, whose `energy_bands` field is **four
coarse bands** (sub / low / mid / high). That is fine for level/balance but
useless for timbre: a plain triangle, a ring-modulated triangle, and a
`math_oscillator` "metallic" carrier produce *nearly identical* 4-band energy
yet sound completely different — the distinguishing information lives in the
**fine partial structure** (where each partial sits, and whether it is harmonic
or inharmonic), which the 4-band metric discards.

The agent also cannot compare its candidate against the **real** SID reference
in the same terms, and cannot see the **per-frame time evolution** that is the
entire point of this class of sound (the spectrum changes every ~20 ms as the
chip switches waveform).

The goal of this plan is to give the agent the tools to run a real fitting loop:

```
target   = spectrum(real SID render)          # measure the goal
candidate = spectrum(my Pertylizer patch)      # measure the attempt
d        = distance(candidate, target)         # how far off, and *where*
            → adjust params to minimise d       # calculate, don't guess
```

This is general-purpose: any "match this sound" / "why does my patch sound
wrong" workflow benefits. The SID work is just the forcing case.

---

## 1. What to build (priority order)

| # | Tool | One-line value | Ship priority |
|---|------|----------------|---------------|
| 1 | `render_to_wav` | Write a (soloable, windowed) offline render to a WAV file. Unblocks *all* external analysis immediately — the agent can FFT it itself and compare to the reference WAV in one pipeline. | **MVP — ship first** |
| 2 | `analyze_spectrum` | In-engine detailed spectrum of a render: detected partials + harmonicity + timbre descriptors. Lets the agent "see" without leaving MCP. | High |
| 3 | `analyze_sample_spectrum` | Same analysis on an *imported sample* (or WAV path) → fingerprint the **real SID reference** in identical units. | High |
| 4 | `compare_spectra` | Distance + per-partial diff between two sources (render↔render or render↔sample). Closes the loop: turns "see it" into a number to minimise. | Medium |

Rationale for the order: **`render_to_wav` alone unblocks the whole method**,
because the agent can run arbitrary DSP (FFT, partial tracking, spectral
distance, even a parameter search) on the WAV in its own environment, and it
already has the reference SID audio (`sidplayfp -u1 -u3 -w …`). `analyze_spectrum`
+ `compare_spectra` then make the loop possible *inside* MCP, which is faster and
keeps the agent from round-tripping files. Build 1 first; 2–4 are independent
follow-ups that can land in any order.

---

## 2. The one reuse that makes this cheap

All four tools sit on top of infrastructure that **already exists** — do not
write a new renderer or pull a new dependency:

- **Offline render → audio buffer:**
  `crates/pertylizer/src/audio/arrangement_render.rs` ::
  `render_arrangement_to_buffer_with_scope(...) -> RenderedArrangement`,
  where `RenderedArrangement { samples: Vec<f32> /* stereo-interleaved L,R */, sample_rate: u32 }`.
  This is exactly what `analyze_mix_bus_impl` (`crates/pertylizer/src/mcp_bridge.rs:9762`)
  uses. Every new tool renders the requested window the same way and then runs
  DSP on `rendered.samples`.
- **Per-instrument / per-track isolation** already exists: the
  `include_per_track` path in `analyze_mix_bus_impl` re-renders each audible
  track soloed. Reuse that soloing mechanism so `analyze_spectrum` /
  `render_to_wav` can target one instrument's contribution rather than the full
  mix. (A clean single-note fingerprint = solo the instrument + a short window.)
- **FFT is already a workspace dependency:** `realfft = { workspace = true }`
  is used by `crates/synth_dsp` and `crates/synth_osc`. Add `realfft` to
  `crates/pertylizer/Cargo.toml` (workspace) — **no new third-party crate.**
- **Sibling analysis fn:** `analysis::energy_bands(&signal, sample_rate)`
  (called from `mcp_bridge.rs`) is where the existing band metric lives. Put the
  new spectral analysis in the **same `analysis` module**, next to it, as pure
  functions over `&[f32] + sample_rate` so they are unit-testable without MCP.

---

## 3. Core DSP layer (new, pure functions — `crates/pertylizer/src/analysis/`)

A new file e.g. `analysis/spectrum.rs`, pure & sample-rate-explicit so it is
testable in isolation (no engine, no MCP). Mono-sum the stereo buffer first
(`(L+R)/2`) unless a `channel` is requested.

**Newtype pattern (mandatory — GEMINI.md "NEVER use raw primitives"):** use the
`synth_core::types` wrappers — `Hertz` (`types/frequency.rs`), `Decibels`
(`types/amplitude.rs`), `Cents` (`types/pitch.rs`), `Seconds` (`types/time.rs`),
`NormalizedValue` (`types/normalized.rs`). They are `#[serde(transparent)]`, so
they serialise to bare floats at the MCP boundary while staying type-safe
in-engine. `f32` stays only for internal FFT-buffer arithmetic, never on a
public field.

```rust
/// A detected spectral peak.
pub struct Partial {
    pub frequency: Hertz,
    pub amplitude: Decibels,        // peak-normalised so the loudest partial = 0 dB
    pub harmonic_number: Option<u32>, // nearest n in n·f0; None when the frame is unvoiced
    pub inharmonicity: Cents,       // signed deviation of frequency from harmonic_number·f0 (0 if None)
}

pub struct SpectrumResult {
    pub f0: Option<Hertz>,          // detected fundamental; None = unvoiced (noise frame)
    pub voiced: bool,               // false → harmonic metrics are not meaningful, treat as noise
    pub partials: Vec<Partial>,     // top-N peaks, descending amplitude
    pub centroid: Hertz,            // spectral centroid (brightness)
    pub flatness: NormalizedValue,  // 0 = pure tone … 1 = white noise (geo/arith mean ratio)
    pub rolloff: Hertz,             // freq below which 85% of energy lies
    pub inharmonicity: NormalizedValue, // aggregate energy-weighted |partial − n·f0| / f0 (0 if unvoiced)
    pub odd_even_ratio: f32,        // Σ odd-harmonic / Σ even-harmonic energy — ratio, dimensionless
    pub bands: AnalyzeEnergyBands,  // reuse the existing 4-band metric for continuity
    pub log_bins: Vec<Decibels>,    // optional: N log-spaced magnitude bins, for full compare
}

pub fn analyze_spectrum(signal: &[f32], sample_rate: u32, opts: SpectrumOpts) -> SpectrumResult;
```

`SpectrumOpts { f0_hint: Option<Hertz>, max_partials: u32 /*default 48*/, log_bins: u32 /*default 0 = off*/, window: WindowFn /*Blackman-Harris default*/, fft_size: usize /*default 8192*/ }`.

Implementation notes — **the short-window precision rules are not optional:**

- **Window → zero-pad → real FFT.** A 20 ms frame at 44.1 kHz is only 882
  samples → raw bin spacing ≈ 50 Hz, far too coarse to resolve harmonics of a
  low f0. **Always window the frame then zero-pad to a power of two
  (`fft_size`, default 8192)** before the `realfft` real FFT. Zero-padding
  interpolates the spectrum so peaks are sharp and parabolic interpolation is
  accurate.
- **Window function.** Default **Blackman-Harris** (sidelobes ≈ −92 dB) so weak
  partials next to strong ones survive; offer Hann (−32 dB) as a faster option.
- **Parabolic peak interpolation (sub-Hz).** For each local max bin `k`, refine
  with quadratic interpolation on the **log-magnitude** spectrum:
  `p = ½·(α − γ) / (α − 2β + γ)`, where `α, β, γ` are log-mags of bins
  `k−1, k, k+1`; true frequency `= (k + p)·Fs/fft_size`. Without this, short
  windows quantise every partial to the coarse bin grid.
  **Guard the denominator** (flat spectrum / silent frame → 0):
  `let p = if denom.abs() > 1e-5 { 0.5*(α−γ)/denom } else { 0.0 };` — otherwise
  rounding or silence yields `NaN`/`Inf`.
- **f0 — robust and fail-safe.** Use **normalized autocorrelation (ACF) or YIN**,
  *not* HPS (HPS collapses on sines / triangles / ring-mod tones with sparse or
  missing fundamentals). When `f0_hint` is given, **don't bypass tracking — use
  it to restrict the lag-search range** (e.g. a fifth either side of the hint
  period). That kills octave-doubling/halving errors while still tracking the
  real pitch (vibrato, detune, bends). **Unvoiced verdict:**
  if the max normalized ACF peak `< 0.4` **or** `flatness > 0.5`, mark the frame
  unvoiced → `f0 = None`, `voiced = false`, skip harmonic tagging (all energy
  goes to inharmonic/noise). This is what keeps **SID noise frames** from
  emitting a garbage f0 that corrupts `inharmonicity` and `harmonic_number`.
- **Partials.** Local maxima above an adaptive noise floor, parabolic-refined,
  sorted by amplitude, truncated to `max_partials`. Tag each (when voiced) with
  nearest harmonic + cents deviation.
- **Edge guards (must not panic / divide by zero):** all-zero (silence) or
  pure-DC buffers → return a well-defined empty/unvoiced result, never NaN in
  centroid/flatness/ACF. **Ignore peaks at or above Nyquist (`Fs/2`)** so
  aliasing artefacts never appear as partials.
- **flatness / centroid / rolloff / inharmonicity / odd_even**: textbook
  formulas over the magnitude spectrum. These scalars are what *separate*
  triangle from ring-mod from metallic where the 4 bands cannot.

Add a `compare(a: &SpectrumResult, b: &SpectrumResult) -> SpectrumDistance`:
```rust
pub struct PartialDiff { pub target_frequency: Hertz, pub target_amplitude: Decibels,
    pub matched_frequency: Option<Hertz>, pub amplitude_delta: Decibels, pub frequency_delta: Cents }
pub struct SpectrumDistance {
    pub log_spectral_distance: f32,         // primary scalar to minimise (over log_bins) — dimensionless
    pub centroid_delta: Hertz, pub flatness_delta: NormalizedValue, pub inharmonicity_delta: NormalizedValue,
    pub missing_partials: Vec<PartialDiff>, // strong in target, absent in candidate
    pub extra_partials: Vec<PartialDiff>,   // present in candidate, not in target
}
```
The `missing_partials` list is the high-value output — it literally says *"the
target has a strong partial at 1153 Hz that your patch lacks,"* which is the
actionable guidance the agent needs.

**Unvoiced comparison rules** (both frames carry the `voiced` flag):
- **both unvoiced** (e.g. two noise frames) → skip partial matching entirely;
  base the distance solely on `log_spectral_distance` over `log_bins`.
- **one voiced, one unvoiced** → apply a **severe distance penalty**: a pitched
  tone vs noise is a gross timbral mismatch, and partial-matching across them is
  meaningless. (Surfacing this as a large `log_spectral_distance` + a flag keeps
  the agent's minimiser from chasing a phantom partial alignment.)
- **both voiced** → the full partial diff above.

---

## 4. MCP wiring (mirror the `suggest_patch_changes` / `analyze_mix_bus` layering exactly)

### 4a. Result types — `crates/synth_mcp/src/types.rs`
Mirror `SpectrumResult` / `SpectrumDistance` as serde-serialisable MCP result
structs (same `From<analysis::…>` conversion pattern already used for
`AnalyzeEnergyBands`).

### 4b. Bridge trait — `crates/synth_mcp/src/bridge.rs`
```rust
fn render_to_wav(&self, path: String, start_tick: Option<u64>, duration_seconds: Option<f32>,
    instrument_id: Option<u16> /*solo*/, sample_rate: Option<u32>) -> Result<RenderToWavResult, McpBridgeError>;

fn analyze_spectrum(&self, start_tick: Option<u64>, duration_seconds: Option<f32>,
    instrument_id: Option<u16> /*solo*/, f0_hint: Option<f32>, max_partials: Option<u32>,
    log_bins: Option<u32>, hop_ms: Option<f32>, window_len_ms: Option<f32>)
    -> Result<SpectrumOrSpectrogram, McpBridgeError>;
// hop_ms set → SpectrogramResult (Section 5); unset → a single SpectrumResult.
// Raw f32 params at the MCP boundary are wrapped into Hertz/Seconds inside the _impl.

fn analyze_sample_spectrum(&self, sample_id_or_path: String, f0_hint: Option<f32>,
    max_partials: Option<u32>, log_bins: Option<u32>) -> Result<SpectrumResult, McpBridgeError>;

fn compare_spectra(&self, a: SpectrumSource, b: SpectrumSource) -> Result<SpectrumDistance, McpBridgeError>;
// SpectrumSource = oneOf { render { window+solo }, sample { id_or_path } }
```

### 4c. Bridge impl — `crates/pertylizer/src/mcp_bridge.rs`
`*_impl` fns dispatched from the trait methods (same as `analyze_mix_bus_impl`):
1. `render_to_wav_impl`: build the render scope (reuse the `analyze_mix_bus_impl`
   scope construction; honour `instrument_id` via the existing solo path), call
   `render_arrangement_to_buffer_with_scope`, write `rendered.samples` to `path`.
   **Reuse the existing `hound` writer — do not hand-roll a WAV header.** `hound`
   is already a `crates/pertylizer` dependency and `audio/export.rs` already
   builds float/int `hound::WavWriter`s (and `bundle.rs` writes
   `SampleFormat::Float`, 32-bit). Factor out / call that path. Return
   `{ path, sample_rate, duration_seconds, channels, peak }`.
2. `analyze_spectrum_impl`: same render → mono-sum (or pick channel) →
   `analysis::spectrum::analyze_spectrum(&signal, sr, opts)` → result type.
3. `analyze_sample_spectrum_impl`: resolve the sample (the sampler already
   decodes WAV/AIFF on `import_sample`; reuse that loader for a raw path, or
   look up an already-imported sample id) → same `analyze_spectrum`.
4. `compare_spectra_impl`: produce both `SpectrumResult`s (render and/or
   sample), call `analysis::spectrum::compare`.

### 4d. Server — `crates/synth_mcp/src/server.rs`
Param struct + handler (`run_blocking_json()`) + dispatch-macro entries +
**tool descriptions** for each, next to `analyze_mix_bus`. Descriptions must
state: offline & deterministic; that `instrument_id` solos one instrument; that
`analyze_spectrum`'s `f0_hint` sharpens harmonic tagging; and that the metrics
(`flatness`, `inharmonicity`, partial list) are what separate timbres the
4-band `analyze_mix_bus` cannot.

---

## 5. The time dimension — native STFT, **O(1) MCP calls (do NOT defer)**

The target sound's identity is that its spectrum **switches every frame**
(~20 ms): triangle-frame vs noise-frame. A single FFT over 2 s averages that
away, so the agent needs per-frame spectra.

> **Do not** have the agent achieve this by calling `analyze_spectrum` 100×
> with 20 ms windows. Each call re-renders the arrangement (graph setup, voice
> re-allocation, MCP round-trip) — 100 renders of a 2 s project freezes the
> loop. That is **O(N) renders for one analysis.**

Instead, **render the requested window once and slide the FFT in Rust.**
`analyze_spectrum` gains optional `hop` + `window_len` (in ms or frames); when
set it returns a spectrogram from the *single* rendered buffer:

```rust
pub struct SpectrogramResult {
    pub time_stamps: Vec<Seconds>,     // window-centre time of each frame
    pub frames: Vec<SpectrumResult>,   // one full SpectrumResult per hop
}
```

So `analyze_spectrum` returns either a single `SpectrumResult` (no hop) or a
`SpectrogramResult` (hop set) — **one render, one MCP call, O(1)**. The
per-frame `voiced` flag (Section 3) then lets the agent read the alternation
directly: voiced triangle frames vs unvoiced noise frames, in one response.

**Pre-allocate, don't allocate per frame.** Build the `realfft` planner, the
window array, the zero-padded input buffer, and the complex output buffer
**once** at the top of `analyze_spectrum_impl`, then reuse them across every
STFT hop. Planning/allocating per frame would dominate the cost; reused
buffers keep the whole STFT well under a few ms for a 2 s project.

**Time-alignment (note for the caller):** because `SpectrogramResult` carries
per-frame `time_stamps`, the agent can align a candidate render against the
target SID WAV by cross-correlating their RMS-energy envelopes *before*
comparing frames — so a small onset/latency offset doesn't smear the per-frame
spectral diff.

---

## 6. Tests — `crates/pertylizer/tests/` (call `*_impl()` directly, repo convention)

DSP unit tests (in `analysis/spectrum.rs`, synthetic signals — no engine):
- `pure_sine_has_one_partial_zero_inharmonicity` — 1 kHz sine → single partial at
  1 kHz, `inharmonicity ≈ 0`, `flatness ≈ 0`.
- `white_noise_has_high_flatness` — flatness near 1, no dominant partial.
- `square_has_odd_harmonics_only` — odd_even_ratio ≫ 1, partials at f0,3f0,5f0.
- `inharmonic_pair_is_flagged` — sine at 1 kHz + sine at 1.41 kHz with f0_hint=1k →
  second partial reports large `inharmonicity` (cents).
- `compare_reports_missing_partial` — target with a 1.5 kHz partial vs candidate
  without → that partial appears in `missing_partials`.
- `unvoiced_noise_returns_none_f0` — white noise → `voiced == false`, `f0 == None`,
  no harmonic tagging (the SID-noise-frame guard).
- `low_f0_short_window_resolves_with_zero_pad` — 100 Hz saw in an 882-sample
  (20 ms) frame zero-padded to 8192 → partials land within a few cents of
  100/300/500 Hz (proves zero-pad + parabolic interpolation; fails on a raw
  882-pt FFT).
- **`silence_and_dc_do_not_nan`** — all-zeros and a pure-DC buffer return a
  finite, well-defined result (no NaN in centroid/flatness/ACF, no panic).
- **`peaks_above_nyquist_are_ignored`** — a tone near `Fs/2` does not emit a
  partial at/above Nyquist.

MCP integration tests:
- `render_to_wav_writes_readable_wav` — render a one-note project, assert the WAV
  exists, has the right sample rate/length, non-silent.
- `analyze_spectrum_solo_isolates_instrument` — 2-instrument project; soloed
  spectrum of the lead differs from the full-mix spectrum.
- `analyze_sample_spectrum_roundtrips` — render_to_wav → import → analyze_sample_spectrum
  matches analyze_spectrum of the same render within tolerance.
- `triangle_vs_metallic_separates_on_descriptors` — the regression that motivates
  this: two carriers whose `analyze_mix_bus` 4-band energy is ~equal must yield
  clearly different `flatness` / `inharmonicity` / partial sets. This is the
  acceptance test for the whole plan.

---

## 7. Out of scope (state in descriptions / defer)

- Parameter *search/optimisation* itself — that lives in the calling agent;
  these tools only **measure** and **compare**. (A future `match_timbre` tool
  could wrap a search, but not now.)
- Psychoacoustic models (Bark/ERB loudness, roughness). Start with the
  standard descriptors above; they already separate the cases that matter.
- Real-time / streaming spectrum. Offline-render-then-analyse only, matching
  `analyze_mix_bus`'s deterministic model.
- AWE / master-effects inclusion flags — copy `analyze_mix_bus`'s existing
  `include_*` flags verbatim if free; otherwise default to the dry instrument
  sum and note it.

---

## 8. Suggested commit sequence

1. `render_to_wav` end-to-end (bridge trait + `_impl` reusing the existing render
   scope + WAV writer + server wiring + `render_to_wav_writes_readable_wav`).
   **This alone unblocks the external math loop** — ship and stop here if time-boxed.
2. `analysis/spectrum.rs` pure DSP (`analyze_spectrum` + `compare`) + its unit
   tests. Self-contained, no MCP.
3. `analyze_spectrum` MCP wiring (types + bridge + `_impl` + server) +
   `analyze_spectrum_solo_isolates_instrument` + the
   `triangle_vs_metallic_separates_on_descriptors` acceptance test.
4. `analyze_sample_spectrum` (reuse the sampler's decode path) + roundtrip test.
5. `compare_spectra` + `compare_reports_missing_partial` integration test.
6. Native STFT spectrogram on `analyze_spectrum` (the `hop`/`window_len` path +
   `SpectrogramResult`) — **core, not optional** (Section 5): one render → many
   frames, so the agent never re-renders per window. Land it with step 2/3 if
   convenient; it shares all the DSP.

Each step keeps `cargo fmt --check && cargo build && cargo clippy --all-targets
&& cargo test` green before committing (per CLAUDE.md), update `docs/history.md`
(one line per tool), and bump `Cargo.toml` per the `new version` flow.

---

## 9. Acceptance — how we know it worked

The motivating agent must be able to:
1. Render the real SID reference to WAV (`sidplayfp -u1 -u3 -w`), import it, and
   get its partial fingerprint (`analyze_sample_spectrum`).
2. Render its candidate patch soloed (`analyze_spectrum`).
3. Call `compare_spectra` and read a `missing_partials` list that names the
   specific frequencies its patch is failing to produce.
4. Adjust patch parameters and watch `log_spectral_distance` fall.

When that loop runs, timbre-matching stops being ear-guessing and becomes a
measured minimisation — which is the entire purpose of this plan.
