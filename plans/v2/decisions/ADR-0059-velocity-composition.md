# ADR-0059: Velocity Composition

| Field | Value |
|---|---|
| ID | ADR-0059 |
| Status | Accepted |
| Phase | 6 |
| Created | 2026-09-06 |
| Last reviewed | 2026-09-06 |
| Related | ADR-0057, ADR-0025, ADR-0007, `SOUND-INV-021`, `SOUND-INV-023`, `LOWER-INV-003`, `P04-R001`, `P06-S004` |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

**Delivered behaviour, and a product choice the user made.** How loud a note of a given
velocity sounds is what a listener hears from every saved project; V1 has one answer and V2,
until this record, another. `P04-R001` carried the difference as a residual from Phase 4:
[ADR-0057](ADR-0057-refuse-parity-verdict-over-a-placed-note.md) refused every parity verdict
over a placed note until Phase 6 stated the composition, and `LOWER-INV-003` is the rule that
holds until it does. It is also **persisted**: V1 saves two sensitivities the composition
reads, and each saved value must keep one meaning.

**Why it is ready now.** `P06-S004` is the next slice and cannot proceed without it, and the
phase's exit requires a parity path for placed notes that this residual blocks.

## Decision boundary

It decides **how many velocity destinations** a voice has, **what each computes** from the
note's velocity and its own sensitivity, and **how V1's saved sensitivities lower**. It does not
reopen ADR-0025's payload — the note carries how hard it was struck, never a scale — nor
SOUND-INV-021's expansion, which already delivers one write per declared destination.

## Evidence

Read from V1's own code, as ADR-0057 verified:

- `synth_modules::math::velocity_sensitivity(velocity, sensitivity)` is
  `1 − sensitivity × (1 − velocity)`; the envelope module multiplies its emitted level by it,
  and its `vel_sens` parameter defaults to `1`.
- `Voice::process` scales the voice's output by `(1 − amp_sens) + amp_sens × velocity`, with
  the instrument's `velocity_amp_sensitivity`, default `1`.
- At both defaults the product is `velocity²`: a saved `0.7559` sounds at `0.572` where a
  single scale sounds at `0.756`, **2.4 dB** apart.
- The instrument's third sensitivity, `velocity_filter_sensitivity` (`velocity_to_filter`,
  default `0.5`), is stored and copied to the voice's expression and **never read by any
  DSP**: it is not part of the composition.
- V2 today applies velocity once, as the envelope's scale, with no sensitivity.

## Options

1. **Reproduce V1 with two destinations.** The voice scope declares two velocity
   destinations, each shaped by its own authored sensitivity: the envelope's, computing V1's
   `1 − s × (1 − v)`, and a voice-output stage computing V1's `(1 − s) + s × v`. The product
   equals V1's for every saved pair, so the lowering's velocity mark comes off placed notes.
   A placed note stays `UnsupportedScope` through Phase 8's own marks — the master volume
   and the pan stages — and a parity verdict stays refused until those are lowered too;
   what this option closes is the velocity clause, not the verdict. Selected.
2. **Apply once and mark lowered notes `Simplified`.** Smallest change; every saved project
   renders up to 2.4 dB louder per note than V1 and no parity verdict claims fidelity.
   Rejected: the phase's exit needs the parity path, and the difference is audible on every
   note.
3. **Reproduce V1 through one destination**, the lowerer pre-composing the two factors into
   the payload's velocity. Rejected: ADR-0025 declined carrying an amplitude scale in place of
   velocity, and a bend or a later per-note scale would then act on the wrong quantity.

## Decision

1. A voice has **two velocity destinations**, each a sample-positioned control that
   SOUND-INV-021's note-on expansion writes, and each with a **sensitivity** of its own: a
   quantum-rate control whose base is authored in the IR.
2. The **envelope's** scale is V1's `1 − s × (1 − v)`, computed by the envelope kernel from
   the velocity it holds and the sensitivity it reads per frame, and applied to the level it
   emits. Its `velocity_sensitivity` is an authored field of the envelope kind.
3. A **velocity scaler** is a voice-scope node kind — audio in, audio out, in place — whose
   scale is V1's `(1 − s) + s × v`, from its own velocity destination and its authored
   `sensitivity`. The lowerer places one between a voice's terminating node and the output,
   carrying the instrument's `velocity_amp_sensitivity`. A plan authored without one has one
   destination, the envelope's, as before.
4. **The formulas are V1's, bit for bit**, so the product at the defaults is V1's `velocity²`
   and a saved project renders its notes at V1's level. A sensitivity of one on the envelope
   leaves V2's existing renders unchanged where the arithmetic is exact and within one
   rounding where it is not; the evidence digests record which.
5. **Lowering.** The envelope module's `vel_sens` lowers to the envelope's sensitivity, the
   instrument's `velocity_amp_sensitivity` to the scaler's, and `velocity_filter_sensitivity`
   is recorded as V1's own dead field: read by nothing, it lowers to nothing and is not a
   fidelity mark. With the composition built, ADR-0057 clause 1's marker is **discharged**: a
   lowering that places a note carries no `Unrepresented` diagnostic for velocity, `LOWER-INV-003`
   is rewritten to the general rule its first sentence already stated, and `P04-R001` closes.

## Falsifier and stopping rule

Violated if a voice with both destinations renders a note at anything but V1's product of the
two factors, if either factor is computed anywhere but its own kernel from its own
sensitivity, if a lowering places a note and still carries the velocity marker, or if any
saved sensitivity lowers to a value other than its own. A differently shaped curve is an
amendment with its arithmetic stated, not a defect.

## Consequences and risks

- **Accepted cost.** One more node kind, one more control on the envelope, one more
  destination in every lowered voice's expansion.
- **Risk: existing V2 renders.** The envelope's scale at a sensitivity of one is V1's
  arithmetic rather than the bare velocity; the two agree exactly for velocities that are
  dyadic and can differ by one rounding otherwise. Control: EVD-0013's digest is re-recorded
  with this reason if it moves.
- **Revisit condition.** A V1 change to either formula, or a Phase 7 modulator on a
  sensitivity.

## Specification update

`SOUND-INV-021` gains the two-destination composition and its formulas; `LOWER-INV-003`
loses its velocity clause and keeps its rule; ADR-0057's clause 1 is recorded as discharged
here rather than rewritten. Written by `P06-S004`.

## Review

Design consultation: the three options were put to the user on 2026-09-06 with the
recommendation first; the user selected option 1. Accepted with that selection.

Independent semantic reviewer: the one independent read of `P06-S004` reviews this record with
the slice.

Stopping rule: a product that is not V1's, a factor computed outside its kernel, or a placed
note still marked blocks acceptance. A digest that moves by rounding, recorded, does not.
