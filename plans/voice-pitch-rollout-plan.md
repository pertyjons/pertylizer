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
yields **13** modules below (`Oscillator` + `Sampler` already done). `pitch_cv`
input ports are **out of scope** — oscillators already have FM/freq-CV inputs.

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
| AdditiveOsc | `additive_osc.rs` | partials already recompute from `base_freq` each block — just feed the modulated base |
| VoiceSynth | `voice_synth.rs` | `note_freq`/`base_freq` drives the source–filter; **vibrato on a voice is high value** |
| AmFormant | `am_formant.rs` | `note_freq` drives the AM-formant synthesis; pitched, vibrato meaningful |
| LaSynth | `la_synth.rs` | PCM+partial pitch |
| FractalOscillator | `fractal_osc.rs` | base freq behind the fractal generator |

### Tier 3 — harder (pitch is woven into grain / formant scheduling; mind smoothing)
| Module | File | Note |
|--------|------|------|
| VocalTract | `vocal_tract.rs` | glottal-source F0; high value (voice vibrato) |
| Fof | `fof.rs` | fundamental F0 drives grain scheduling + per-band `phase_inc` |
| GranularOsc | `granular_osc.rs` | pitch is grain playback rate, not a single phase increment |

## Quality concerns

- **Click/zipper:** a phase-accumulator oscillator is click-free by construction
  — only the phase *increment* changes, the phase stays continuous (Tier 1 and
  most of Tier 2 are safe). The risk is in **resampling / grain** modules (Tier 3,
  WavetableOsc) where a block-rate pitch jump can step audibly under fast
  modulation. For those, smooth the pitch toward the target across the block (or
  per sample) rather than snapping. Note any module that snaps.
- **Control rate:** `set_voice_pitch` is called once per block. That matches the
  existing oscillator behaviour and is fine for glide/vibrato at audio-block
  granularity; per-sample pitch is not required.
- **Behaviour-preserving for static notes:** a held note with no
  glide/vibrato/bend must sound exactly as before — the per-block value then
  equals the `note_on` pitch (assert this in tests).

## Phasing

One module per commit, gated (`cargo build` / `clippy --all-targets` / `test` /
`fmt`), reviewable independently. Suggested order = value × ease:

1. **Phase 1 (Tier 1):** SubOsc, MathOsc, PadSynth, WavetableOsc, RingMod.
2. **Phase 2 (Tier 2):** VoiceSynth + AdditiveOsc first (melodic value), then
   AmFormant, LaSynth, FractalOsc.
3. **Phase 3 (Tier 3):** VocalTract, Fof, GranularOsc — with smoothing care.

Stop after any phase if priorities change; each module is independently complete.

## Testing

Per module, a behavioural regression test mirroring the sampler's
(`voice_pitch_modulates_sampler_playback_speed`):

1. `note_on` at frequency `F`; process a block; estimate the output fundamental.
2. `set_voice_pitch(2F)`; process a block; assert the fundamental **doubled**
   (within tolerance).
3. Assert that *without* `set_voice_pitch` the output stays at `F` (static-note
   no-regression).

Add a shared test helper that estimates a block's fundamental by zero-crossing
rate (sufficient for the simple oscillators); Tier 3 may need a coarse FFT-bin
peak instead. Keep the existing sampler/oscillator tests green.

## Verification (after the code)

In-app ear-check: build a patch on each converted source, enable portamento and
per-note vibrato, and confirm the pitch actually bends — the same standing
ear-check the tracker importer needs.

## Out of scope / open

- `ChaoticOsc` / `Fooglers` note-pitch decision — making them note-pitched at all
  is a separate change from continuous-pitch (see Excluded). `Fooglers` (a
  click-excited waveguide) is the more defensible candidate to become pitched.
- `pitch_cv` input ports on the oscillators (they already have FM/freq-CV; revisit
  only if a modular pitch-CV path is specifically wanted).
- Per-sample (vs per-block) pitch smoothing — only if a Tier 3 module audibly
  steps; otherwise block-rate is the contract.
