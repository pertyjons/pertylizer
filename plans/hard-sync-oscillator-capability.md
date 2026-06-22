# Plan: audio-rate oscillator hard sync

## Priority — what to build first (read this before anything else)

This is a **tiered** plan. **Only Tier 1 is recommended for the initial
implementation.** Tiers 2–3 are opt-in quality / robustness work, to be added
*only if* the spectral verification loop (or real general-purpose use) shows
they are needed. Do **not** build them speculatively — that is exactly where the
maintenance cost and complexity live.

Why this is worth doing at all (general value, not a SID-only obscurity): hard
sync is one of the signature techniques of subtractive synthesis (sync leads /
sync sweeps across Prophet-5, Minimoog, Serum, Vital…), and the oscillator's
**own `sync` port description already promises it** — *"Connect: another
Oscillator's output for hard sync sounds"*. This completes an already-intended,
half-wired feature; SID export is the motivating trigger, not the justification.

- **Tier 1 — minimal, do this first (satisfies SID export).** Retype the
  oscillator's `sync` input from `gate` to a plain **`audio`** input, and replace
  the current gate-threshold reset with **wrap detection on the master** (a large
  negative sample-to-sample delta) followed by a **sample-aligned** phase reset.
  The SID-export master is a fixed-frequency **sawtooth**, so a drop of `> 0.5`
  between consecutive samples unambiguously marks its cycle wrap (the ripple from
  band-limiting is far smaller than the ~2.0 wrap drop). Footprint: one port
  retype + ~10 lines inside one `process()` loop that already has `prev_sync` and
  a reset path. **Per the V2 review, retyping `sync` to a plain audio input lets
  the connection validator accept `audio → audio` with no new type rule** — so
  the type-checker work my earlier draft listed disappears (verify `can_drive`
  has no `sync`-specific special-casing).

- **Tier 2 — opt-in audio quality (defer until the spectral loop asks for it).**
  Sub-sample reset (`(1−t)·dt` instead of snapping to zero) to remove sample-grid
  pitch jitter, plus a band-limited step (`poly_blep`/`minblep` residual) at the
  reset discontinuity. **Gate both on `aa_mode`:** `AntiAliasMode::Raw` skips them
  and snaps to `Phase::ZERO` — which is precisely the gritty, aliased, authentic
  SID sound, so Raw mode delivers SID fidelity *for free* while PolyBlep/MinBlep
  modes stay a clean general-purpose tool.

- **Tier 3 — opt-in robustness (only if syncing from arbitrary masters).** Add a
  dedicated **phase/ramp output** (Option A′). Amplitude-independent, no waveform
  coupling, exact sub-sample `t` recovery (math below). Worth it for
  general-purpose sync from any source/level; **unnecessary for the SID export**,
  whose master is a known fixed saw at full level.

Everything below is the supporting detail; the tier list above is the decision.

## Implementation effort & sequencing (grounded in oscillator.rs)

"Correct from the start, no shortcuts" ≠ "build all three tiers now." The
architecturally *correct, non-hacky* baseline is **Tier 1 + Tier 3**, and it is
cheap. **Tier 2 (BLEP) is a strictly additive quality layer on the same reset
point, gated on `aa_mode` — deferring it costs no rework**, because
`generate_single_sample` (L214) already computes the *naive* waveform
(`Phase::new_unchecked(p).sawtooth()`) before applying existing BLEP, so the
`Δ = naive(p_old) − naive(p_new)` extraction is trivial when the time comes.

| | Tier 1 | + Tier 3 (correct arch) | + Tier 2 (band-limited) |
|---|---|---|---|
| What | retype `sync`→audio (L484); replace gate logic (L561–569) with negative-delta wrap detect | + phase-ramp output (new port + buffer + writer near L617) + exact `(1−t)·dt` sub-sample reset | + BLEP residual at the sync step |
| LOC | ~7 | ~22 | ~80–120 |
| Files | 1 | 2 (osc + `interned.rs` for `PortName::PHASE`) | 2–3 |
| New state | 0 | 1 (phase buffer) | +1–2 (BLEP carry / insertion buffer) |
| Risk | trivial | low, mechanical | medium–high, fiddly, needs tuning |
| Effort | <½ day | ~1 day total | +2–4 days |

Where the real DSP cost sits — three things visible in the source, all Tier 2:
1. **Cross-sample state.** The sync step's BLEP residual spans the sample
   boundary, but `generate_single_sample` is `&self`; the correction must live in
   the loop via a `&mut self` carry field → new field + `reset()` handling
   (L710).
2. **Not a drop-in.** Existing `poly_blep(p, dt)` is keyed to the waveform's *own*
   wrap (`p ≈ 0`/`1`), not an arbitrary sync fraction `t`; needs a fractional
   `poly_blep_frac(t)` helper and care against double-correcting when the slave's
   own wrap coincides with the reset.
3. **MinBLEP-at-sync** needs a multi-tap insertion buffer (the `MINBLEP_TABLE` is
   multi-sample) — the part most likely to balloon. Scope it to polyBLEP-only at
   the sync point first; document that `aa_mode = MinBlep` uses polyBLEP there.

Sequencing: ship Tier 1 + Tier 3 together (~1 day). Then let the spectral loop
decide Tier 2 — for the SID goal, `aa_mode = Raw` is already the authentic aliased
SID timbre, so the export may match the reference with no BLEP at all.

## Why

The SID-analyzer synth-native export reproduces SID timbres as Pertylizer
patches via derived rules (ring mod → `rng`, tonal↔noise/tonal alternation →
LFO-gated dual source, PWM/filter sweeps → automation). **Hard sync is the one
common SID technique with no Pertylizer path.** A census over HVSC #84 puts it
at ~2.9% of audible frames (≈20.6k notes / 169k frames in a 639k-note sample) —
currently exported as a plain oscillator, dropping the characteristic bright,
sweeping, inharmonic sync timbre entirely.

SID hard sync: oscillator *n*'s phase is reset whenever the **previous** voice
(*n−1*)'s oscillator completes a cycle. The slave's perceived pitch is pulled
toward the master's fundamental and its spectrum gains the inharmonic sync
sidebands. It is an **audio-rate phase reset**, fixed inter-voice routing —
structurally the sibling of ring mod (also voice *n* ← *n−1*).

## What's blocked today (verified via MCP + source audit, 2026-06-22)

The oscillator already exposes a `sync` **input** port, and the DSP node already
*does* a phase reset on a rising edge — but neither the typing nor the mechanism
supports audio-rate hard sync:

- `osc.sync` is typed **`gate`** (`get_module_type_info("osc")`); gate inputs
  accept only `gate`/`control`. `check_connection(osc.out → osc.sync)` →
  `valid:false`, "Incompatible signal types: audio output → gate input." No audio
  source can drive it.
- The current reset (oscillator.rs `process()`, ~L564) is **gate-threshold,
  sample-aligned, reset-to-zero**:
  ```rust
  if sync_val > 0.5 && self.prev_sync.as_f32() <= 0.5 {
      for phase in &mut self.unison_phases[..voice_count] { *phase = Phase::ZERO; }
  }
  ```
  A note-retrigger on a `gate` edge — not an audio-rate, sub-sample reset. (Tier 1
  must replace the `> 0.5` rising-edge test, which is wrong for a bipolar audio
  signal, with negative-delta wrap detection.)
- The oscillator's only outputs are `audio` (`out`/`out_l`/`out_r`) — **no
  phase / ramp output** a slave could lock to unambiguously (Tier 3 adds one).
- **No module emits a `gate` signal**, and there is no dedicated sync/clock/phase
  module.

## DSP correctness: what audio-rate sync actually requires

A first review raised aliasing and sub-sample timing; the source audit corrects
two of its premises but confirms its core concerns (and the V2 review concurs):

- **The oscillator is NOT a naive aliasing accumulator.** It already band-limits
  every waveform: `AntiAliasMode::{PolyBlep, MinBlep, Raw}`, PolyBLAMP for
  triangle, a DSF path — `poly_blep(p, dt)`, `poly_blamp(...)`,
  `Self::minblep_correction(p, dt)` all already exist in oscillator.rs, with a
  precomputed `MINBLEP_TABLE`. `prev_sync` (L87) and `dt`
  (`freq.phase_increment(sample_rate)`) already exist.
- **But the existing BLEP does not cover a sync reset.** Those corrections cancel
  each waveform's *intrinsic* edges; a mid-cycle phase reset injects a **new,
  independent step discontinuity** none of them touch. Snapping to `Phase::ZERO`
  at the sample clock both (a) quantises the sync instant → **phase jitter** and
  (b) leaves an un-band-limited step → **sync aliasing**. This is what Tier 2
  fixes (when `aa_mode != Raw`).

### The edge-detection trap (justifies Tier 1's delta test and Tier 3)

Detecting a **zero-crossing of the master audio** (the first review's sketch) is
unreliable as a cycle marker: a square/pulse master has no smooth zero crossing;
triangle/sine cross zero **twice** per cycle; a band-limited saw rings/overshoots
(Gibbs) near its wrap → can cross zero more than once → **double-syncs**. The
robust target is the master's **phase wrap**, not a generic zero crossing —
exactly how the real SID syncs on accumulator overflow regardless of waveform.

Tier 1 handles this cheaply *because the SID master is always a saw*: a wrap is a
single large **negative delta** (`sync_val − prev_sync < −0.5`), well clear of the
small band-limit ripple. Tier 3 generalises it to any master via a clean ramp.

### Tier 3 math — exact sub-sample wrap recovery from a phase ramp (V2)

With a dedicated 0→1 phase ramp output as the master sync signal, the slave reads
`x[n]` (this sample) vs `x[n-1] = prev_sync` and recovers the wrap fraction
`t ∈ [0,1)` exactly, **without knowing the master frequency**:

```text
wrap detected when   x[n] < x[n-1]
dt_master = x[n] - x[n-1] + 1.0           # true master increment across the wrap
t         = (1.0 - x[n-1]) / dt_master    # fraction of the [n-1, n] interval at wrap
p_slave   = (1.0 - t) * dt_slave          # restart at the wrap instant, advance the remainder
```

The phase ramp is monotone, ripple-free (no Gibbs), and amplitude-independent —
which is exactly why it removes the edge-detection trap. (For Tier 1's saw input
you can approximate `t` from the saw's own slope, or skip `t` entirely and accept
the sample-aligned reset until Tier 2.)

### Tier 2 — band-limiting the reset step (V2, with one caveat)

Treat the reset as a generic step of height `Δ = y_slave(p_old) − y_slave(p_new)`
and add a band-limited residual around the discontinuity (split across samples
`n`/`n+1` by the fraction `t`).

> Caveat the V2 review glosses: the existing `poly_blep(p, dt)` corrects a step at
> the *waveform's own* wrap (`p ≈ 0`/`1`). A sync reset at an arbitrary fraction
> `t` needs a BLEP evaluated at fractional delay `t` — a small helper, or the
> table-based `minblep_correction` parameterised by `t`. It is **not** a literal
> drop-in of `poly_blep(p, dt)`; the V2 "reuse `poly_blep`, scale by `Δ/2`" tip is
> the right *idea* (poly_blep is sized for a step of 2.0) but still needs the
> fractional-delay evaluation, and care not to double-correct when the slave's own
> natural wrap coincides with the reset.
>
> Out of scope: master frequencies so high that multiple wraps land within one
> sample (near Nyquist). Cap to one reset per sample and degrade gracefully — not
> relevant to audio-range SID masters.

## Capability options (mapped to the tiers above)

**A — audio-rate `sync` input on the oscillator.** Tier 1. Retype `sync` to
`audio`, detect the master wrap, reset the slave. Mirrors the ring-mod solution
one-to-one (master `osc` fixed at the *n−1* frequency, `key_track`/note-track off
like the `rng` carrier → slave `osc.sync`). Constrain the SID-export master to a
**sawtooth** so its wrap is one unambiguous negative-delta edge per cycle.

**A′ — A plus a dedicated phase/ramp output.** Tier 3. The phase output is the
ideal sync source (monotone, no overshoot, amplitude-independent) and gives the
exact `t` above. Folds the most valuable bit of Option C into A without a new
signal type. Recommended *only* once arbitrary-master robustness is wanted.

**B — dedicated hard-sync / master-slave module.** Not recommended. Master+slave
in one `process()` loop gives the wrap fraction exactly, but it is less modular
(can't sync from an arbitrary source) and duplicates oscillator waveform/AA code.
A `SidSyncOscillator` coupling 2–3 accumulators would be phase-accurate for SID
only — more surface than A′ for narrower benefit.

**C — new `sync` signal type + output.** Not recommended. Most general but the
largest surface change (`PortType::Sync`, `can_drive`, connection UI). A′ captures
its value for far less, and Tier 1's `audio → audio` retype needs *no* type-system
work at all.

## SID fidelity vs. band-limiting (resolved cleanly by `aa_mode`)

Real 6581/8580 sync is a sample-clocked accumulator overflow with no
band-limiting — gritty and aliased by nature. So `AntiAliasMode::Raw` (skip
sub-sample + BLEP, snap to `Phase::ZERO`) is not a degraded mode here, it's the
**authentic SID** mode; `PolyBlep`/`MinBlep` give the clean general-purpose tool.
The existing per-mode AA switch already carries this distinction, so Tier 2 adds
no new user-facing concept.

## Export side (sid-analyzer, already prepared)

`HardwareTrick::HardSync` is already detected per note
(`analysis/timbre/characteristics.rs`). The ring-mod path is the exact template:
`ring_source_frequency(span, voice_idx, clock)` captures voice *n−1*'s freq at
ring-bit frames → `NoteCharacteristics.ring_source_hz` →
`PatchVoiceProfile.ring_source_hz` → drives `rng-1.carrier_freq`. For hard sync,
add `sync_source_hz` (identical `(voice_idx + 2) % 3` previous-voice capture,
gated on the sync bit) and, once Tier 1 lands, emit a fixed-freq master
oscillator (**sawtooth** waveform; or its phase output under Tier 3) → slave
`osc.sync`, routed from the raw master output **before** any VCA/filter (level/DC
must not corrupt the trigger). No new analysis is needed — only the Pertylizer
capability and ~30 lines of export wiring.

## Verification (the proven spectral loop)

1. Render real SID hard-sync reference: `sidplayfp -u… -t<n> -w` soloing the sync
   voice; `analyze_sample_spectrum` → record the inharmonic sideband cluster.
2. Build the candidate (master → slave sync) via MCP; `render_to_wav` +
   `compare_spectra` voiced-frame-to-voiced-frame; minimise `log_spectral_distance`
   (the method that validated the Nemesis V2 ring-mod carrier). **This loop is
   also the gate for whether Tier 2 is needed at all** — if Tier-1 `Raw`-mode sync
   already matches the SID reference, do not build the BLEP path.
3. DSP-specific checks (only relevant once Tier 2/3 exist):
   - Sweep the slave far above the master; compare the high-band alias skirt with
     `aa_mode = Raw` vs `PolyBlep`/`MinBlep` — the BLEP residual should drop the
     floor and track `aa_mode`.
   - Hold master+slave fixed; confirm the synced pitch is stable (no jitter
     sidebands) — that verifies the `(1−t)·dt` sub-sample reset.
   - Drive sync from non-saw masters (sine/triangle/square) — confirm no
     double-trigger (the Tier 3 phase output should make this clean).

Until Tier 1 lands, hard-sync SID tunes export as a plain oscillator (no
regression, just unrealised fidelity).
