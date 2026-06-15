# Audit: `set_mod_offset` coverage across voice modules (2026-06-15)

**Trigger:** while fixing oscillator pitch destinations (control-script #12-B) it
emerged that the oscillator *silently dropped* `detune`/`frequency` mod offsets.
This audit checks whether the same gap exists elsewhere.

**Verdict: it is pervasive.** S1.1's headline promise — "the Mod Matrix can target
**any** modulatable param on any module" — is **not actually delivered**. The
address picker (GUI + MCP) offers any descriptor param with `modulatable == true`,
but a routing only does anything if the target module's `set_mod_offset` has a
`match` arm for that param's `type_id`. Otherwise it hits `_ => {}` and **silently
vanishes** — the routing validates, the marker lights, it persists, and it does
nothing.

## The numbers

- **40** voice (`PolyModule`) modules.
- **4** override `set_mod_offset` at all (`amplifier`, `filter`, `lfo`,
  `oscillator`) — the other **36 inherit the no-op default**, so *every* modulatable
  param on them is dropped.
- Even the 4 that override handle only **2–4** targets each and drop the rest.

## Gap table (modulatable float params dropped)

### Partial coverage — override `set_mod_offset` but drop some targets

| Module | Handled | **Dropped (silent)** |
|---|---|---|
| oscillator | pitch, detune, frequency, level | **pulse_width (PWM!)**, fm_amt, x_mod (cross-mod) · *structural: unison, uni_detune, uni_spread, uni_phase* |
| filter | cutoff, resonance | **drive**, env_amt, key_track, cv_amt, morph |
| amplifier | level, pan | master, cv_bipolar |
| lfo | rate, depth | phase |

### Zero coverage — no `set_mod_offset` (ALL params dropped) — sound shapers (high priority)

| Module | Dropped (all modulatable) |
|---|---|
| sub_osc | level |
| noise | level |
| ring_mod | carrier_freq, mix, freq_ratio, key_track |
| wavetable_osc | position, detune, level · *octave* |
| additive_osc | tilt, odd_even, brightness, stretch, randomize, level |
| formant_filter | vowel, cutoff, resonance, mix |
| vocal_tract | tongue, constriction, lips, length, nasality, breathiness, level |
| voice_synth | vowel, formant_shift, breathiness, open_quotient, tilt, vibrato_rate/depth, unison_*, level |
| fof | vowel, formant_shift, skirt, bandwidth, breathiness, vibrato_rate/depth, unison_*, level |
| math_oscillator, chaotic_osc, fractal_osc, granular_osc, padsynth, la_synth, am_formant, fooglers, body_resonance, mechanical_noise, sampler | (all float params — 3–11 each) |

### Zero coverage — modulator / utility modules (judgment needed)

- **Modulator sources** whose params *are* meaningful destinations (modulate an
  envelope's decay, an MSEG rate, etc.): `envelope` (8 floats), `mseg`, `lfo`
  (partial), `envelope_follower`, `drift_generator`, `kinetic_modulator`,
  `turing_machine`, `random_gates`, `euclidean`.
- **Genuinely N/A** (mod-destination meaningless — should likely be
  `.modulatable(false)` so the picker stops offering them): `output`,
  `signal_monitor`, `audio_input`, `mod_matrix`, `pitch_tracker`, `beat_detector`,
  `vector_mixer` / `keyboard_panner` (maybe), and the *structural* params noted
  above (`unison` voice count, `octave` step, `randomize` seed, …).

## Two honest fixes per dropped param

1. **Implement** — add a `set_mod_offset` arm that applies the normalized offset
   to the param's effective value (musical scale per the param's unit).
2. **Mark `.modulatable(false)`** — for params where mod is meaningless/structural,
   so the picker stops offering a dead target.

Every modulatable param must end up in (1) or (2). Right now ~150 params are in
neither — silently dropped.

## Strategic options for the fix (NEEDS A DECISION)

This is **not** a mechanical per-module loop — it's an architecture choice:

- **A — Generic default mechanism (recommended for scale).** Give `PolyModule` a
  default `set_mod_offset` that stores normalized offsets in a per-`type_id` map,
  plus a helper `effective(param) = denormalize_through_curve(base_norm + offset)`
  that modules call when reading a param. Realizes the *already-locked* scaling
  contract (option A: normalized-through-range) generically — **one** implementation
  covers every param. Cost: each module's param reads must route through the helper
  (mechanical but touches every module). Per-param musical exceptions (osc pitch =
  semitones, filter cutoff = ±48) stay as overrides.
- **B — Per-module hand-wiring.** Add `set_mod_offset` arms module-by-module for the
  params that matter; mark the rest non-modulatable. ~40 modules, judgment per param,
  many commits. Pragmatic but slow and inconsistent (every module invents its own
  scale, the exact thing the scaling contract wanted to avoid).
- **C — Triage + shrink.** First mark all non-musical floats `.modulatable(false)`
  (kills the dead picker entries cheaply), then hand-wire only the high-value
  shapers (osc PWM, filter drive, sub/wavetable/ring/noise level, formant/vowel
  params). Smaller than B, leaves a curated set.

**Recommendation: A** (it's the scaling contract finally realized, and the only
option that scales to 40 modules without 40 inconsistent hand-rolled scales). If A
is too big a refactor to take on now, **C** is the pragmatic interim.
