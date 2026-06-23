# Plan: roll out continuous voice pitch to the remaining source modules

## Goal

Make **every pitched voice source** respond to continuous voice-pitch modulation
(glide / vibrato / pitch-bend / mod-matrix `OscPitch`), not just `Oscillator` and
`Sampler`. Today the other source modules freeze their pitch at `note_on`, so a
patch built on (say) a wavetable osc silently ignores portamento, per-note
vibrato, and any LFO→OscPitch routing. That inconsistency is the bug; this plan
closes it.

This is **engine/DSP quality work, not importer work.** It is what makes the
tracker importer's vibrato/glide audible on sampled *and* synth instruments, but
it stands on its own.

## The mechanism is already in place (do not rebuild it)

- `PolyModule::set_voice_pitch(&mut self, freq: Hertz)` — trait method, **default
  no-op** (`synth_core/src/module_traits.rs:1175`).
- `Voice::process_audio` calls `graph.set_voice_pitch(freq)` **every block**
  (`voice.rs:940` → `graph.rs:501`), dispatching to **all** modules.
- `freq` is the voice's already-resolved modulated pitch — note + glide + vibrato
  + bend + mod-matrix folded into **one** value. A module does **not** implement
  any effect; it just stops freezing its pitch and tracks the commanded value.
- Reference impls: `Oscillator` (`oscillator.rs:756`, the trivial case — set the
  stored frequency) and `Sampler` (`sampler.rs:404`, recomputes playback speed
  from `freq`, gated on `pitch_tracking`).

Each module's task is therefore uniform: **"on `set_voice_pitch`, update the
field `process` reads for pitch, instead of only setting it in `note_on`."**

## Scope — the source modules and their grading

**Membership test** (verified exhaustively against the codebase, not memory): a
module is in scope iff it is **category `Oscillator`** *and* its `note_on` sets
pitch from the note (`self.note_freq = note.to_frequency()` or equivalent). That
yields **12** modules below (`Oscillator` + `Sampler` already done). `pitch_cv`
input ports are **out of scope** — oscillators already have FM/freq-CV inputs.

> Every in-scope module's `note_on` was re-verified to set `note_freq` from the
> note; a DSP review caught one that does *not* (`LaSynth`) — now Excluded.

### Excluded (and why — all verified)
- **`ChaoticOsc`** (`chaotic_osc.rs`) — category **LFO**; `note_on` is an empty
  no-op. Not note-pitched by design.
- **`Noise`** (`noise.rs`) — category Oscillator but `note_on` ignores the note
  (only resets state). Not pitched.
- **`Fooglers`** (`fooglers.rs`) — category Oscillator, but `note_on` injects a
  click excitation and **ignores the note**. It's a click-excited waveguide whose
  pitch comes from a parameter, not the note. ⚠ It arguably *should* be
  note-pitched (Karplus-style), but that is a **separate change** (making it
  note-pitched at all), not continuous-pitch — out of scope here, flagged.
- **`LaSynth`** (`la_synth.rs`) — category Oscillator, but `note_on` **ignores
  the note** (`_note`); its transient generators use **hardcoded** frequencies
  (pluck sweep 1200→800 Hz, hammer 300 Hz) and the sustain stage is a passthrough
  of `PortName::IN` (there's even a `test_passthrough_without_note`). There is no
  pitch state to update, so `set_voice_pitch` would be a no-op. Making it
  note-pitched (scaling the transient sweeps to the note) is a separate feature,
  not this rollout. *(Caught by DSP review; the plan previously mislisted it in
  Tier 2.)*
- **`MechanicalNoise` / `BodyResonance` / `KeyboardPanner`** (PhysicalModeling) —
  none is a note-pitched source: noise trigger, an input resonator, and a
  note→pan mapper respectively. Out of scope.

### Tier 1 — easy (store-a-base-frequency, ≈ identical to `Oscillator`)
| Module | File | Pitch field today | `set_voice_pitch` does |
|--------|------|-------------------|------------------------|
| SubOscillator | `sub_osc.rs` | `base_frequency` (set in `note_on`) | set `base_frequency` |
| MathOscillator | `math_oscillator.rs` | `frequency` / `base_frequency` | set `frequency` |
| PadSynth | `padsynth.rs` | `phase_increment` (note is pitch-only; table is baked & stays) | recompute `phase_increment` from `freq` |
| WavetableOsc | `wavetable_osc.rs` | base freq behind `effective_freq()` | set the stored base freq |
| RingMod | `ring_mod.rs` | `note_freq` (carrier; set in `note_on`) | set `note_freq` — carrier sidebands track the note |

### Tier 2 — medium (pitch fans out to many derived values; recompute per block)
| Module | File | Note |
|--------|------|------|
| AdditiveOsc | `additive_osc.rs` | partials already recompute from `note_freq` each sample — just feed the modulated base |
| VoiceSynth | `voice_synth.rs` | unison voices recompute phase increments per sample from `note_freq`; **vibrato on a voice is high value** |
| AmFormant | `am_formant.rs` | `note_freq` drives the AM-formant synthesis; pitched, vibrato meaningful |
| FractalOscillator | `fractal_osc.rs` | `note_freq` scales the fractal generator |

### Tier 3 — harder (pitch is woven into grain / formant scheduling; needs care)
| Module | File | Note + required care |
|--------|------|------|
| VocalTract | `vocal_tract.rs` | glottal-source F0; high value (voice vibrato). **⚠ amplitude discontinuity:** `inc = note_freq·inv_sr` is per block (L434) and the excitation is `(flow − prev_flow)/inc` (L481) — a derivative *divided by* `inc`, so a block-rate `inc` jump steps the excitation amplitude → click under fast modulation. **Mitigation: linearly ramp `inc` across the block** — hold `current_inc`, compute `d_inc = (target_inc − current_inc)/block_len`, and `current_inc += d_inc` inside the sample loop (one extra add/sample, negligible) so the excitation derivative stays smooth. Effectively required here, not optional. |
| Fof | `fof.rs` | fundamental F0 drives grain scheduling + per-band `phase_inc`; recompute from `note_freq`, watch for the same divide-by-`inc`-style normalization. |
| GranularOsc | `granular_osc.rs` | **⚠ contract decision:** each grain captures `rate` at spawn (`Grain.rate`, L52); modulating pitch only affects *new* grains, so active grains don't bend → with long grains, fast vibrato smears into a "pitch trail"/cluster. Lush as a granular feature, wrong for instrument glides. **Decide:** (a) keep trails (cheap, do nothing) *or* (b) true continuous bend. **⚠ bend can't be a naive rate swap:** today `read_pos = (start_pos + pos·rate)` (L534) with `pos += 1` per sample, so making `rate` dynamic rescales the *whole* elapsed offset → an instantaneous read-position jump (phase tear / crackle, momentary reverse). Bend therefore needs a **`Grain` refactor to an accumulating playhead**: store a per-grain `pitch_offset_ratio` (not absolute rate), keep `pos` for the envelope, and advance `read_pos += base_rate·pitch_offset_ratio` per sample (`read_pos` is the new accumulated field). More than a one-liner — that's the cost of option (b). |

## Quality concerns

`set_voice_pitch` is called **once per block**, and the engine's default block is
**256 samples** (`graph.rs`, ≈ 5.3 ms / ≈ 188 Hz block rate at 48 kHz) and is
resizable upward. So pitch changes are step functions at block boundaries — large
enough blocks that steep/fast modulation is audible. Three distinct severities:

1. **Phase-accumulator oscillators (Tier 1 + VoiceSynth/AdditiveOsc/MathOsc):**
   click-free by construction — only the phase *increment* steps, the phase stays
   continuous. The only artefact is block-rate FM sidebands, inaudible for gentle
   vibrato (≈6 Hz over ~31 updates/cycle), faintly audible only under steep bends
   or fast LFO. **Smoothing optional** — skip unless an ear-check flags it. *(The
   review suggested smoothing VoiceSynth too; it's the same benign FM-sideband
   class as Tier 1, so it's optional there, not required.)*
2. **Amplitude-discontinuity modules (VocalTract, and any module that *divides*
   by the per-block pitch step):** here a stepped `inc` steps the output
   *amplitude*, not just adds sidebands → an actual click. **Smoothing /
   per-sample `inc` effectively required** (see Tier 3).
3. **Grain-capture modules (GranularOsc, partly Fof):** pitch is latched per grain
   at spawn; the question isn't zipper but the trail-vs-bend contract (Tier 3).

- **Behaviour-preserving for static notes:** a held note with no
  glide/vibrato/bend must sound exactly as before — the per-block value then
  equals the `note_on` pitch (assert this in tests).

### Real-time safety
`set_voice_pitch` runs on the audio thread (inside `Voice::process_audio`), and
each module is owned there (`Box<dyn PolyModule>` per voice graph node). So the
pitch fields are **plain `f32`/`Hertz`, not atomics** — there's no cross-thread
sharing to guard, which keeps the struct compact and cache-friendly. The impls
must stay alloc-free and panic-free (plain field writes + arithmetic) per the
project's RT rules. Note: the call is a **virtual dispatch** through the
`dyn PolyModule` vtable, so `#[inline]` on the impl does **not** cross it — but
it's one vtable call per module per *block* (control rate), which is negligible;
don't add `#[inline]` expecting it to help here.

## Phasing

One module per commit, gated (`cargo build` / `clippy --all-targets` / `test` /
`fmt`), reviewable independently. Suggested order = value × ease:

- [x] **Phase 0 — test infra:** shared **AMDF** estimator + `render_mono` harness
  (`voice_pitch_harness.rs`), validated end-to-end against `Oscillator`.
- [x] **Phase 1 (Tier 1):**
  - [x] SubOsc
  - [x] MathOsc
  - [x] PadSynth
  - [x] WavetableOsc
  - [x] RingMod
- [ ] **Phase 2 (Tier 2):**
  - [x] VoiceSynth
  - [ ] AdditiveOsc
  - [ ] AmFormant
  - [ ] FractalOsc
- [ ] **Phase 3 (Tier 3):**
  - [ ] VocalTract (per-sample `inc`)
  - [ ] Fof
  - [ ] GranularOsc (*after* deciding the trail-vs-bend contract)

Stop after any phase if priorities change; each module is independently complete.

## Testing

Per module, a behavioural regression test mirroring the sampler's
(`voice_pitch_modulates_sampler_playback_speed`):

1. `note_on` at frequency `F`; process a block; estimate the output fundamental.
2. `set_voice_pitch(2F)`; process a block; assert the fundamental **doubled**
   (within tolerance).
3. Assert that *without* `set_voice_pitch` the output stays at `F` (static-note
   no-regression).

**Pitch estimator — build this first, before Phase 2.** Zero-crossing rate is
*not* robust for the harmonic-rich / formant-heavy / noise-coupled outputs
(AdditiveOsc, VoiceSynth, VocalTract, Fof) — harmonics cross zero several times
per cycle and wreck the estimate. Use an **AMDF** (Average Magnitude Difference
Function) detector in the shared helper instead:
`AMDF(τ) = Σ |x(n) − x(n−τ)|`, with the pitch period = the `τ` minimizing it over
a **bounded** lag window around the expected period (e.g. ±35%:
`lag ∈ [0.65·P, 1.35·P]` where `P = sample_rate/expected_hz`). The bound both
speeds it up and avoids octave-jump errors; normalize each lag's sum by its
overlap count (`N − τ`). It's cheap, a few lines of Rust, and robust against
formant peaks and harmonic shift — one helper for every tier, living in a shared
test module. Keep the existing sampler/oscillator tests green.

## Verification (after the code)

In-app ear-check: build a patch on each converted source, enable portamento and
per-note vibrato, and confirm the pitch actually bends — the same standing
ear-check the tracker importer needs.

## Out of scope / open

- **Make excluded modules note-pitched at all** — `LaSynth` (scale its transient
  sweeps to the note), `Fooglers` (Karplus-style: note → delay length), and
  `ChaoticOsc`. Each is a separate *feature* (giving the module note pitch),
  distinct from this rollout (continuous pitch for already-note-pitched sources).
  `Fooglers` and `LaSynth` are the more defensible candidates.
- **GranularOsc trail-vs-bend contract** — decide before Phase 3 (Tier 3).
  Default leaning: continuous bend, since the importer's instrument glides want a
  coherent sweep, not a cluster — but it requires the accumulating-playhead `Grain`
  refactor (see Tier 3), not a one-liner. If that cost isn't worth it, trails is a
  legitimate (do-nothing) fallback.
- `pitch_cv` input ports on the oscillators (they already have FM/freq-CV; revisit
  only if a modular pitch-CV path is specifically wanted).
- Per-sample (vs per-block) pitch smoothing — only if a Tier 3 module audibly
  steps; otherwise block-rate is the contract.
