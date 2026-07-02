# SID Oscillator module (`sid`) — implementation instruction

> **Status: proposed (2026-07-01).** A new voice-graph module that reproduces the
> MOS 6581/8580 waveform generator (SID) — combinable waveforms, the noise LFSR,
> ring/hard-sync, TEST, and the 6581/8580 DAC — instead of a general band-limited
> musical oscillator. Motivation: the `sid-analyzer` project exports C64 SID tunes
> to Pertylizer; the timbral core (combined waveforms, ring-mod, `$D418` digi)
> cannot be reproduced by the general `osc` module. An AudioScript PoC vs a reSID
> reference reached 47 dB → 16 dB log-spectral distance; a native chip-accurate
> module is the path to reSID-level fidelity and collapses the export to a near-direct
> register mapping. See `sid-analyzer/docs/pertylizer-sound-engine-plan.md`.
>
> **Expert-reviewed twice 2026-07-02.** Review 1 (DSP / 6581-8580): clock-domain jitter
> handling (§2), combined-waveform physics + parametric-model option C (§2/§7), TEST →
> `0x7FFFFF` LFSR reload (§2/§6), ring-mod is triangle-only (§3), hard-sync must be
> band-limited (§3), data-license isolation (§7). **Review 2 (integration seams / Rust),
> all verified against the source and folded in:** the real factory is
> `pertylizer/src/module_factory.rs::create_voice_module`, **not** `commands.rs`
> (which is a partial 6-type factory) — §1; `ModuleType` must be **appended** to the
> enum, not inserted — §1; the top-level `Param` enum is in `params/mod.rs:695` and
> `module_param.rs` is only the trait-impl macro — §1; the trait method is
> `descriptor()`, not `describe()` — §1/§5; bool params are **not** automatable
> (`is_automatable` needs `Continuous`) so waveform bits are `.modulatable(false)` — §4;
> a ring/sync **source** oscillator needs `TrackVoicePitch`/`FreqReg` because
> `set_voice_pitch` broadcasts the played note to every module — §3/§4; SID ring/sync
> topology must stay **acyclic** (the graph forbids cycles) — §3; `note_on` must **not**
> reseed the LFSR/accumulator (they free-run; TEST does) — §5; the sample-rate rule was
> over-strict — a refreshed cache mirror is fine — §6; raw register params use `u32`
> (`u16` has no `ScalarParam`) — §4; and the LFSR feedback polynomial (`bit22 XOR bit17`)
> is now specified — §2. A prior-draft error (seeding noise from the voice index) was
> corrected: SID noise is deterministic from register state.
>
> **Design note 2026-07-02 (scriptability).** An earlier "absolute-set CV" idea was
> dropped: it diverged from Pertylizer's uniform additive-CV convention. CV is now
> **additive only** (like every module) — the exporter expresses absolute PW/pitch as
> offset-from-base — and per-frame **waveform sequences live in a module-internal
> `WaveformSequence` param**, not a divergent CV. General discrete-param step-automation
> is noted as an optional separate engine feature, not a dependency (§3 *Scriptability*, §4).

## 0. Compatibility verdict (why no engine changes are needed)

A SID oscillator is a **new `PolyModule` that reuses the exact scaffolding the current
`Oscillator` already has** — per-voice phase state, an audio-rate `sync` input, a
`phase` output, pitch-from-note, per-sample generation, and `sample_rate` handling.
Everything SID-specific lives inside the module's `process`. Concretely, the engine
already provides every hook:

- **Per-voice model + lifecycle:** the `PolyModule` trait
  (`crates/synth_core/src/module_traits.rs`) — `process` (:13), `note_on` (:39),
  `set_voice_pitch(freq)` (:54), `reset` (:36), `set_sample_rate` (:68),
  `set_voice_index` (:172), `box_clone` (:227).
- **Pitch:** `set_voice_pitch(Hertz)` (:54) delivers the live note frequency
  (glide/bend included) — the module maps it through the SID master clock.
- **Cross-voice ring/sync WITHOUT cross-voice engine access:** the SID's "previous
  voice" ring/sync map to **two `sid` modules wired inside one instrument graph**,
  exactly like the existing hard-sync-via-two-oscillators pattern. The current
  `Oscillator` already takes an **audio-rate `sync` input** (`oscillator.rs:86-90`,
  `prev_sync`) and emits a clean **`phase` output** (`oscillator.rs:119-122`,
  `phase_buffer`). A `sid` module exposes the same plus a `ring` input.
- **Sample rate:** `ProcessContext.sample_rate` (`module_traits.rs:340`) is the source
  of truth. Caching a private `sample_rate` mirror for coefficient/state is fine **iff**
  it is refreshed from `context.sample_rate` at the top of every `process` (the pattern
  `oscillator.rs` uses). The AudioScript bug was *not refreshing* it — not the caching
  itself (see `plans/audioscript-sample-rate-and-context-vars.md`).

So this is "register a new `ModuleType` + implement `PolyModule` + add a param enum".
No change to the graph, voice manager, port system, render path, or serialization
core is required. **One caveat (§4/§7):** the ring/sync *source* oscillator needs a
module-level `TrackVoicePitch`/`FreqReg` param so it can hold the neighbour voice's
pitch instead of the played note — that's a param on the new module, still not an
engine change. The work is the DSP and the wiring boilerplate below.

## 1. Integration checklist (files + anchors)

Mirror the `RingMod` (`rng`) / `AudioScript` (`asc`) precedents — both are recent
single-module additions and touch exactly these seams.

1. **`ModuleType` variant** — `crates/synth_core/src/params/mod.rs` (enum at `:144`):
   - **append `SidOscillator` at the END of the enum**, not near `Oscillator`. The enum
     is `#[serde(rename_all)]`-serialized and inserting mid-enum shifts every later
     variant's discriminant / ordering — append is the safe convention (that's why
     `RingMod`/`AudioScript` sit late in the list). **[review-2 blocker]**
   - `display_name` (near `:405`), `prefix() => "sid"` (near `:486`, cf. `rng`/`asc`),
     `from_prefix()` (near `:569`), the **voice-category** match arms (`:265-322`), and
     the `with_f32` round-trip arm (near `:1115`).
2. **Param enum** — new `crates/synth_core/src/params/sid_oscillator.rs` defining
   `SidOscillatorParam` (mirror `OscillatorParam`, `params/oscillators.rs:596`). Then
   **three distinct seams** (the earlier draft conflated them — **[review-2 blocker]**):
   - add a `SidOscillator(SidOscillatorParam)` variant to the top-level **`Param` enum,
     which lives in `params/mod.rs:695`** (NOT `module_param.rs`);
   - add the new type to the **`impl_module_param!` macro list in
     `params/module_param.rs`** (that file is only the blanket trait-impl aggregation);
   - add `ScalarParam` impls for its value types in `params/scalar_impls.rs` /
     `kind_impls.rs`; re-export the enum from `params/mod.rs`.
3. **Factory — the REAL one** — `crates/pertylizer/src/module_factory.rs`
   `create_voice_module`: add a `ModuleType::SidOscillator => { … }` arm
   (cf. `RingMod` at `:72`). **[review-2 blocker]** `synth_engine/src/commands.rs::create_module`
   is a *partial* `ModuleTypeId`-string factory (6 basic types, `None` otherwise) that
   RingMod/AudioScript do **not** use — do not register there and expect it to work.
   `module_factory.rs` is exercised by its own factory test, so a missing arm fails
   loudly.
4. **Module implementation** — new `crates/synth_modules/src/sid_oscillator.rs`
   implementing `PolyModule` + `Describable`; `pub use sid_oscillator::SidOscillator`
   in `crates/synth_modules/src/lib.rs` (cf. `:102`).
5. **Descriptor** — implement **`descriptor()`** (the `Describable` trait method,
   `module_traits.rs:1264`; it is `descriptor()`, **not** `describe()` — **[review-2]**)
   returning `ModuleCategory::Oscillator` (cf. `oscillator.rs:327`) + the ports/params
   from §3–§4.
6. **Schema mirror** — after it compiles, the module count + descriptor JSON change;
   re-sync any schema mirrors (the `sid-analyzer` repo keeps a copy under
   `docs/pertylizer/`).

## 2. DSP core — the SID waveform generator

Do **not** reuse the float `Phase` (`types/normalized.rs:453`) or the analog
`generate_sample` (`oscillator.rs:220`) — SID waveforms are derived from an
**integer accumulator**, and the combined-waveform tables + noise LFSR index its
raw bits. Implement a chip-faithful core per voice:

- **Accumulator:** a 24-bit phase accumulator (`u32`, masked to 24 bits). Each SID
  clock, `acc = (acc + freq_reg) & 0xFF_FFFF`. `freq_reg` is the 16-bit SID frequency
  register derived from the note (see §5).
- **Clock-domain conversion — must be jitter-free (expert-review §2).** The SID clock
  is **not** an integer multiple of the host rate (PAL `985248/44100 ≈ 22.341`, NTSC
  `≈ 23.191`). Stepping a *fixed* integer number of SID sub-cycles per host sample
  (round to 22/23) injects audible low-frequency **phase jitter** and distortion,
  worst at high pitch. A naive host-rate "raw" oscillator also over-brightens (the
  reason our `raw` PoC overshot reSID; measured). Use one of:
  1. **Fractional cycle stepping + BLEP:** advance the accumulator by a fixed-point
     fractional number of SID cycles per host sample; when a discontinuity crosses
     (pulse edge, saw wrap, sync reset), compute its exact fractional position within
     the sample and insert a band-limited step (PolyBLEP/MinBLEP) there. No 1 MHz loop.
  2. **True 1 MHz stream + polyphase decimation (reSID method):** run the chip at its
     exact integer clock and decimate with a high-quality polyphase FIR / sinc
     resampler that tracks the continuously-shifting fractional ratio. Highest
     fidelity, highest CPU.
  3. **Hybrid (recommended MVP):** run the accumulator at host rate with SID pitch
     quantization and PolyBLEP the *pure* waveforms + sync transients (cheap, no 1 MHz
     loop); **oversample+decimate only the combined-waveform and noise paths**, whose
     many small steps / table structure PolyBLEP can't clean. Expose the strategy /
     oversample factor as a `Quality` param.
- **Sawtooth:** top 12 bits of the accumulator (`acc >> 12`) as a rising ramp.
- **Triangle:** MSB (bit 23) XORed across the next 11 bits, then `<< 1`, MSB dropped
  — the classic saw→triangle fold. Half the amplitude resolution of the saw.
- **Pulse:** `(acc >> 12) >= pw_reg ? high : low`, `pw_reg` a 12-bit register (0–4095).
- **Noise:** a 23-bit Fibonacci LFSR clocked from accumulator **bit 19** (shift on its
  0→1 rising edge). **Feedback polynomial (specify it or the LFSR is ambiguous /
  untestable — review-2):** on each shift, `new_bit0 = bit22 XOR bit17`, then shift the
  register left by one (`sr = ((sr << 1) | new_bit0) & 0x7F_FFFF`). The 8-bit output is
  assembled from taps bits 22,20,16,13,11,7,4,2. Noise *pitch* thus tracks the
  frequency register.
  **Init/reset the LFSR to `0x7F_FFFF` (all 23 bits set), NOT `0` (expert-review §4).**
  An all-zero LFSR is stuck forever (no feedback can ever set a bit). SID noise is
  deterministic from the register state + accumulator, so reproducibility comes from
  the fixed `0x7FFFFF` seed, **not** from a per-voice `set_voice_index` seed — do
  **not** decorrelate noise by voice index (that was unfaithful in an earlier draft;
  `set_voice_index` is irrelevant to a chip-accurate SID voice).
- **Combined waveforms (the headline).** When >1 waveform bit is set the outputs meet
  on a shared internal bus. It is **not** a clean digital AND, and the physics differs
  by model (expert-review §3): the **6581** pulls a common DAC bus low through analog
  switches — a voltage-divider/bus-conflict that is strongly non-linear and even
  varies with chip voltage/temperature/production batch; the **8580** is closer to a
  logical AND but still carries DC-offset effects. So per-model handling is mandatory.
  Three implementation options (pick per §7 licensing):
  - **(A) Measured tables** — per (combination, model) an 8-bit output indexed by the
    top 12 accumulator bits (the reSID `wave*_PST/PS/PT/ST` arrays). Highest fidelity,
    but the tables are the GPL-sensitive artifact.
  - **(B) Algorithmic AND + 6581 leakage** — bitwise-AND with a partial-bit-pulldown;
    cheap, ~the PoC's 16 dB, adequate-ish for 8580, weak for 6581.
  - **(C) Parametric resistor-network / threshold model (recommended)** — reSIDfp's
    approach: a small model (a handful of fitted coefficients, not a big table) trained
    against measurements reproduces the 6581 non-linearity faithfully. Best fidelity/
    license trade-off: the *coefficients* are tiny and can be independently derived, so
    no GPL table is shipped. Pure math for 6581 without full circuit sim was called
    "nästan omöjligt" in review — the parametric model is the practical answer that
    isn't a raw table.
  - **Noise+other = LFSR corruption ("NoiseLock"):** enabling noise together with any
    other waveform pulls LFSR bits low (NMOS bus conflict) → the LFSR locks into a very
    short cycle / silence until TEST/reset. Model by forcing the affected LFSR bits to
    0 while a combination is active. Real, deliberately-used behaviour.
- **TEST bit:** zeroes and holds the accumulator (silences the oscillator) and
  **reloads the noise LFSR to `0x7F_FFFF`** (expert-review §4), not zero. Used for
  hard-restart / phase alignment; the deterministic `0x7FFFFF` reload is what makes
  drum/short-noise effects sound identical on every playback.
- **DAC non-linearity:** apply the per-model waveform-DAC curve (6581 non-linear R-2R,
  8580 near-linear). Part of the "SID character"; a small lookup or the reSIDfp DAC
  model.

## 3. Ports

Reuse the established audio-rate cross-oscillator pattern (`oscillator.rs`):

| Port | Dir | Type | Purpose |
|------|-----|------|---------|
| `out` | out | audio | The DAC'd waveform output. |
| `msb` | out | audio | The accumulator **MSB as a gate** (0 while bit 23 clear, 1 while set) — the exact signal SID sync/ring read. **Do NOT just reuse the existing `phase` 0→1 ramp** (review-2): SID hard-sync triggers on the MSB **0→1 rising edge**, whereas the current osc's `sync` detects a *wrap* (a negative 1→0 jump, `oscillator.rs`). Either emit a real MSB gate and detect its 0→1 edge, or define exact thresholding `msb = phase >= 0.5` with rising-edge detection — but pick one and state it, don't leave "msb or phase" ambiguous. |
| `sync` | in | audio | Hard-sync master: reset this accumulator on the source's **MSB 0→1 rising edge** (not on wrap). |
| `ring` | in | audio | Ring source: the neighbour voice's MSB (same signal as `msb` out). |
| `fm` / `pwm` | in | control | **Additive** pitch / pulse-width modulation — standard CV convention, like every other module (see *Scriptability* below). |
| `test` | in | gate | TEST / hard-restart, script- or gate-driven (a trigger input — a normal convention, no divergence). |

Ring/sync are fed by a **second `sid` module in the same instrument**, tuned to the
neighbour SID voice's captured pitch (`ring_source_hz`) via `TrackVoicePitch=false` +
`FreqReg` (§4) — **not** by the played note, because `ModuleGraph::set_voice_pitch`
broadcasts the played frequency to *every* module in the graph (`graph.rs:501`) and a
plain oscillator overwrites its own frequency from it (`oscillator.rs:781`).

Three DSP/topology specifics (review-1 §5–§6, review-2):
- **Ring mod affects ONLY the triangle.** Apply `MSB(this.acc) XOR MSB(ring_src)` to
  the **triangle folding direction** — not to saw/pulse/noise. If triangle isn't
  selected, the RING bit has no audible effect (unless triangle is combined in).
  Implement the XOR strictly inside the triangle generator.
- **Bandlimit the hard-sync reset.** Zeroing the accumulator mid-sample is a
  discontinuity that aliases badly. At host rate (§2 strategy 1/3) insert a
  PolyBLEP/MinBLEP (or DPW) residual at the sync point; strategy 2 handles it inherently.
- **Keep the source oscillators ACYCLIC (review-2).** The engine graph is topologically
  sorted and **forbids cycles** (`graph.rs:645`). The real SID's V1↔V2↔V3 ring/sync ring
  is cyclic — do **not** try to wire the three exported voices into that topology; it
  will fail the cycle check. Model each ring/sync consumer with a *dedicated, one-way
  source `sid`* (modulator → consumer, no back-edge). Full 3-voice cross-coupling, if
  ever wanted, belongs in a future **`sid_chip` aggregate module** that owns all three
  accumulators internally (out of scope here).

### Scriptability — additive CV only, no divergent semantics

**Design principle: this module obeys the same CV convention as every other module —
CV is *additive* modulation.** An earlier draft proposed an "absolute-set" CV mode
(CV replaces a register); that was rejected. Mixing absolute and additive CV in one
module is a hidden divergence — nothing in a port's type tells a patcher whether it
adds or replaces — and it earns its keep nowhere, because the SID's per-frame writes
can be expressed *without* it. Keep the module conventional; push the "absolute value"
problem to where it's cheap.

**Continuous per-frame modulation (PW, pitch) — additive, solved in sid-analyzer.**
A SID driver writes absolute PW/freq every frame, but *absolute is representable as
additive-from-a-known-base*: the exporter sets the base `PulseWidthReg`/`FreqReg` param
and emits `target − base` as the CV offset each frame. `pwm`/`fm` therefore behave
exactly like `osc.pwm`/`osc.fm` (additive around the param), driven by a `scr` (`arr`
table by `age`/frame) or an ordinary automation lane. No new semantics.

| Target | CV input | Type | Convention | Driven by |
|--------|----------|------|------------|-----------|
| Pulse width | `pwm` | control | additive around `PulseWidthReg` | `scr`/`asc`/lane, offset-from-base |
| Pitch | `fm` | control | additive around `FreqReg` | `scr`/`asc`/lane, vibrato/slide/offset |
| Hard-restart | `test` | gate | trigger | `scr`/gate, per-frame TEST |
| Ring / sync | `ring` / `sync` | audio | see §3 | a second `sid`, not a script |

**Discrete waveform sequences — module-internal program data, NOT a CV.** The one thing
additive CV can't express is a per-frame *waveform-mask* sequence (an offset on a 4-bit
mask is meaningless), and the mask params are bools which aren't automatable anyway
(`is_automatable` needs `Continuous`, `module_traits.rs:899`). Rather than diverge with
an absolute nibble CV, **the module owns the waveform sequence as its own data**: a small
`WaveformSequence` param (a short array of 4-bit masks) + a `WaveformRate`, clocked by the
module's internal frame counter (§4). This is self-contained, chip-honest (the waveform
table *is* part of the SID instrument program), needs **no CV divergence and no engine
change** — the exporter just fills the param. Do **not** split a sequence into
per-waveform note segments: that resets the accumulator phase between segments
(un-chip-faithful) and re-introduces the special-case zoo the whole redesign kills.

> **Optional general alternative (not required here):** if Pertylizer later wants
> per-frame sequencing of *discrete* params for **all** modules, the clean generalization
> is engine-side **step-automation of enum/bool params** (relaxing the `Continuous`-only
> rule in the automation system). That's a bigger, reusable change — flagged as a separate
> item, not a dependency of this module. Until/unless it lands, the internal
> `WaveformSequence` param is the SID osc's answer.

What a script **cannot** reach either way: the internal DSP (accumulator math, LFSR
feedback, combine tables, DAC) — the fixed timbre core, intentionally not exposed.
Scripting the oscillator at that level is the `asc`-as-oscillator alternative (which
forfeits the chip-accurate tables) — a different trade, see the sound-engine plan.

## 4. Parameters (`SidOscillatorParam`)

| Param | Kind | Range / choices | Notes |
|-------|------|-----------------|-------|
| `Triangle` / `Sawtooth` / `Pulse` / `Noise` | Bool ×4 | on/off | **Combinable waveform mask** — the key departure from `osc`'s exclusive `Waveform` enum (`params/oscillators.rs:596`). **Set `.modulatable(false)`** (review-2): `is_automatable()` requires `ParamKind::Continuous` (`module_traits.rs:899`), so bools are **not** sequencer-automatable anyway — they're register bits for MCP/GUI/register-set, not ramped lanes. Static/base setting; per-frame sequences use `WaveformSequence` below, not automation. |
| `WaveformSequence` | data (array of u8 nibbles) | 0–15 per step, N steps | **Per-frame waveform-mask program**, owned by the module and clocked by its internal frame counter — the chip-honest home for a waveform sequence (no CV divergence, no automatable-bool path needed). When non-empty it drives the mask each frame; when empty the static mask bits apply. The exporter fills this from the driver's captured waveform table. |
| `WaveformRate` | Enum/Continuous | frames per step (default 1 = 50 Hz PAL) | Advance rate of `WaveformSequence`; 1 step/frame is the canonical SID case. |
| `FreqReg` | Integer (u32) | 0–65535 (16-bit) | Raw SID frequency register. **Do not reuse the `PulseWidth`/`Hertz` float newtypes** — they're 0.01–0.99 / Hz-log and don't fit register semantics. `u32` already has a `ScalarParam` impl (`params/scalar_impls.rs`); `u16` does **not** (review-2). Used when `TrackVoicePitch=false` (see below). |
| `TrackVoicePitch` | Bool | default **true** | **[review-2, the ring/sync-source fix]** When true, `set_voice_pitch` sets `FreqReg` from the played note (normal voice). When false, the module **ignores** `set_voice_pitch` and holds its own `FreqReg` — this is how a ring/sync *source* `sid` stays tuned to the neighbour voice's `ring_source_hz` instead of the played note (`graph.rs:501` broadcasts to all modules). |
| `PulseWidthReg` | Integer (u32) | 0–4095 (12-bit) | Raw 12-bit PW register. **Not** the `osc` float `PulseWidth` (0.01–0.99). |
| `Test` / `RingMod` / `HardSync` | Bool ×3 | on/off | Control bits, `.modulatable(false)`. RingMod/HardSync also need the `ring`/`sync` input wired. |
| `Model` | Enum | `6581` / `8580` | Selects combined-waveform model + DAC curve. |
| `Clock` | Enum | `PAL` / `NTSC` | Master clock for freq mapping (§5). |
| `Quality` | Enum/Integer | strategy / oversample factor | §2 clock-domain strategy. |
| `Level` | Continuous (Gain) | 0–1 | Output trim (cf. `OscillatorParam::Level`); the one genuinely automatable param. |

Param plumbing mirrors `OscillatorParam`: `ScalarParam`/`kind()`/`with_f32`/`as_f32`/
display names (`params/oscillators.rs` + `params/kind_impls.rs` + `params/scalar_impls.rs`).
Register-valued params use the `u32` `ScalarParam` (integer kind); only `Level` (and
the additive `fm`/`pwm` amounts) are `Continuous`/automatable. **`.modulatable(false)`
on the register bits does not mean "unmodulatable":** per-frame **PW and pitch** arrive
as **additive CV** on `pwm`/`fm` (offset-from-base, driven by a `scr` script or a lane),
and per-frame **waveform** changes come from the module-internal **`WaveformSequence`**
param — not from any absolute/divergent CV. See the *Scriptability* subsection.

## 5. Pitch / clock mapping

- `set_voice_pitch(freq: Hertz)` (`module_traits.rs:54`) delivers the live note
  frequency. **Only when `TrackVoicePitch=true`** (§4), convert it to `FreqReg`:
  `freq_reg = round(f_hz * 16777216 / f_clock)`, `f_clock` = 985248 (PAL) / 1022727
  (NTSC) — reproducing the chip's pitch quantization. When false, ignore the call and
  keep the authored `FreqReg` (the ring/sync-source case).
- **`note_on` does NOT reseed the LFSR or reset the accumulator (review-2).** SID
  oscillator/noise state free-runs independently of the musical gate; the LFSR reload
  to `0x7FFFFF` and the accumulator zeroing are tied to the **TEST / hard-restart
  register sequence** (§2), not to note-on. `note_on` may still latch gate for anything
  gate-driven, but it must not touch accumulator/LFSR phase.
- The accumulator increments per **SID clock**, so the per-host-sample advance is
  `freq_reg * (f_clock / sample_rate)` spread across the oversampled sub-steps (§2).

## 6. Sample-rate correctness + determinism

- `context.sample_rate` (`module_traits.rs:340`) is the **source of truth**. A cached
  `sample_rate` mirror (for coefficient/state) is fine **iff** refreshed from
  `context.sample_rate` at the top of every `process` and coefficients recomputed on
  change — the pattern `oscillator.rs` uses. The AudioScript bug was *failing to
  refresh* the mirror, not the caching itself
  (`plans/audioscript-sample-rate-and-context-vars.md`).
- **Determinism comes from the fixed `0x7FFFFF` LFSR init**, not `set_voice_index`.
  A chip-accurate SID voice is fully deterministic from its register state + accumulator;
  do not decorrelate the noise per voice (that's a unison/analog idiom, not SID
  behaviour). Offline renders are then bit-reproducible for the analysis tools by
  construction. `set_voice_index` (`module_traits.rs:172`) is available but unused here.

## 7. Licensing decision — isolate the data (expert-review §7)

reSID is GPL; reSIDfp is LGPL. To protect Pertylizer's core license from GPL
contamination, **do not embed reSID arrays in a core crate.** Instead:

- **Isolate all chip data behind a boundary** — a dedicated crate (e.g. `sid-tables`)
  or an external data file the user supplies/downloads, that the `sid` module reads.
  The core stays license-clean regardless of how the data is sourced.
- **Prefer the parametric model (§2 option C)** for the combined waveforms: its fitted
  coefficients are tiny and can be **independently derived** from measurements, so
  nothing GPL is shipped while still reaching 6581 fidelity. This is the best
  license/fidelity trade-off and the recommended default.
- Fallbacks: derive equivalent tables by sampling a permissively-licensed emulator or
  measuring hardware; or use freely-licensed data (e.g. VICE, if its license permits).
- Ship the algorithmic AND + leakage (§2 option B) as the zero-data MVP baseline.

**Take this decision formally before any reSID array is copied.** It's a project/legal
decision, not a technical blocker.

## 8. Testing

- **Golden reSID A/B (the primary gate).** The `sid-analyzer` repo already has the
  harness: `mk_sid.py` builds single-note PSID fixtures per waveform/combination,
  `sidplayfp -w` renders the reSID reference, and the Pertylizer MCP `compare_spectra`
  reports `log_spectral_distance` + missing/extra partials. Target: each canonical
  fixture (saw, pulse+PW, tri, each combined pair, ring-mod pair, hard-sync pair,
  noise) within a chosen dB bar of reSID, per model. Combined pulse+tri must recover
  the even-harmonic family (110/220/330…) the general `osc` misses.
- **Unit tests** on raw waveform values: saw ramp, triangle fold, pulse threshold at
  a known PW, LFSR sequence for a fixed seed, TEST zeroing. Assert 6581 vs 8580
  combined tables differ.
- **Render-path pitch test** (mirror `voice_pitch_harness.rs`): a plain saw at MIDI 45
  renders `f0 ≈ 110 Hz` at both 44.1 kHz and 48 kHz (guards the SR-correctness rule).

## 9. Rollout

- **MVP:** accumulator + saw/tri/pulse + 12-bit PW + hard-sync (audio `sync` in) +
  `msb` out + PAL/NTSC + oversample/decimate + `Level`. No noise, no combine tables,
  no DAC yet. Already beats `osc` for basic SID voices and validates the port/param
  wiring end-to-end against the harness.
- **Phase 2:** noise LFSR + NoiseLock; ring-mod input; combined-waveform tables
  (option A or B) per model; TEST bit.
- **Phase 3:** 6581/8580 DAC curves; `$D418`-style digi hooks if desired (may live in
  the amp/volume path, out of scope here).

## 10. Downstream (sid-analyzer)

Once shipped, the SID→Pertylizer export replaces its `asc`-combine, summed-2-osc, and
internal-carrier `rng` approximations with a direct register mapping onto one `sid`
module per voice (plus a second `sid` for ring/sync neighbours). Track that in
`sid-analyzer/docs/pertylizer-sound-engine-plan.md` (Tier-3 becomes native).

## 11. §8 golden A/B — FIRST FULL MATRIX MEASURED (2026-07-02, sid-analyzer session)

The widened fixture matrix now exists in
`sid-analyzer/assets/fixtures/sound-engine-poc/` (`mk_sid.py` regenerates it:
controls {0x21,0x31,0x51,0x61,0x71} × pitches {A1 55, A2 110, A4 440 Hz} ×
{6581, 8580 via header flags — sidplayfp honors them} + noise 0x81, ring-mod
tri@B5 (osc3 165 Hz), hard-sync saw@C#5 (osc3 110 Hz)). Method: reSID WAV vs
soloed MCP render, `compare_spectra` windowed 500–1500 ms. All numbers are
`log_spectral_distance` in dB (lower = closer); the per-pitch **saw row is the
method floor**.

| control            | 6581 A1 | A2   | A4    | 8580 A1 | A2   | A4     |
|--------------------|--------:|-----:|------:|--------:|-----:|-------:|
| 0x21 saw (floor)   |     2.7 |  6.8 |  10.7 |     4.5 |  9.5 | 26.0 ¹ |
| 0x31 tri+saw       |  26.1 ² | 23.7 |  13.1 |    13.9 | 12.6 |   16.0 |
| 0x51 pulse+tri     |     9.3 | 10.2 |  11.6 |     8.4 |  8.1 |    9.9 |
| 0x61 pulse+saw     |    89.7³| 32.1 |  30.8 |     6.7 |  7.5 |    9.7 |
| 0x71 saw+tri+pulse |   115 ⁴ | 114 ⁴| 175 ⁴ |    12.7 | 12.1 |   10.4 |
| 0x81 noise         |       — | 0.76 |     — |       — | 0.77 |      — |
| ring-mod (B5)      |         | 18.9 |       |         | 18.8 |        |
| hard sync (C#5)    |         |  6.8 |       |         |  7.0 |        |

**What's already right:** noise LFSR essentially perfect (0.76 dB); hard sync at
the floor on both models; pulse+tri (the musically dominant combo) at the floor
everywhere; ALL 8580 combos at/near floor (option B ≈ plain AND is enough for
the 8580).

**Two real 6581 gaps (both in `combine_bus`, `sid_oscillator.rs:356` — the
option-C seam):**
1. **0x31 tri+saw:** the real 6581 *kills the fundamental* — reference RMS is
   ~26 dB below saw (124 vs 2567) with energy at high accumulator-bit products
   (strongest partials 8·f0/16·f0); the module keeps a full-level spectrum with
   a strong f0 → 26/24/13 dB.
2. **0x61 pulse+saw:** the real 6581 pulls the bus to **near-silence**
   (reference RMS ≈ 6 = noise floor, reads unvoiced); the module renders a loud
   voiced waveform → 32/31 dB. Note 0x71 *is* modelled near-silent (candidate
   unvoiced too — rows marked ⁴ are silence-vs-silence, where the metric is
   meaningless but the **character agrees**); 0x61 should collapse likewise but
   doesn't.

**Smaller findings:** ring-mod lands the sideband frequencies exactly
(988±165 → 1152.3/822.8 measured) but is ~1.4–1.5 kHz **too bright** vs the
chip's ring-modded triangle (18.9 dB both models) — suspects, in order: the
fold XOR (`waveform_12bit`, `sid_oscillator.rs:437`) switches direction
instantaneously at the ring source's MSB edge (the analog chip's transition is
slewed through the DAC, softening the discontinuity's HF), and the ring path's
oversampling factor. Repro: `ring_b5_{6581,8580}.sid/.wav` fixtures + a
tri/RingMod/`Track Pitch` off `sid`×`sid` candidate; gate = this matrix row
< ~10 dB. Also observed: 6581 combined tri+pulse carries **DC ≈ −0.22** at the
module output (physically plausible for pulled-down combined levels, and reSID
is AC-coupled downstream) — consider a one-pole DC blocker on the module output
or a doc note, it eats mix headroom. ¹ the 8580 saw A4 26 dB is anomalous: zero centroid/partial
error, so the distance lives in the **empty bins between harmonics** (noise
floor mismatch at sparse-harmonic pitches) — likely a measurement-floor
artifact, not a waveform error; check before chasing it. ² ³ see gaps above.

These measurements are exactly the independently-derived data §7 wants for
fitting option C without touching reSID arrays: fit the 6581 combine to the
{0x31, 0x61} references (3 pitches each) and re-run this matrix as the gate.

### §11 addendum (2026-07-02, late) — seq LOOP flag needed by the exporter

The sid-analyzer export now emits `sid` voices end-to-end (combined masks, native
ring/sync with neighbour sources, chip-noise drums, `pw_reg` lanes — exporter-path
A/B matches the hand-built matrix: saw 5.4 dB, combined 10.7 dB). Two engine asks
surfaced:

1. **Waveform-sequence loop mode.** `seq_step` playback HOLDS at the last step
   (`sid_oscillator.rs:311`, `.min(seq_length-1)`) — a one-shot program. But the
   canonical SID alternation (Nemesis V2 tri↔noise) REPEATS for the whole note, and
   detected loop bodies can exceed 16 steps only trivially (Nemesis's is a 22-frame
   body that is pure period-2 after frame 1). Add a **`seq_loop` bool param**
   (`idx = pos % len` when set); the exporter then replaces its two-LFO amplifier
   gate + dual-sid graph with one native sequence. Hold stays the right default for
   drum-attack programs.
2. **Is `pw_reg` automatable?** The export writes an automation lane on
   `sid_oscillator.pw_reg` (398 points on the Nemesis PWM voice). It loads and lists
   (`module:sid:1:pw_reg`) but the descriptor says `modulatable: false` — confirm the
   lane actually reaches the register at render time; if not, mark `pw_reg` (and
   `freq_reg`?) automatable. PWM is a core SID idiom, so this lane must be live.
