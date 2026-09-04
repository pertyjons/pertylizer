# ADR-0007: Parameter Modulation Laws

| Field | Value |
|---|---|
| ID | ADR-0007 |
| Status | Accepted |
| Phase | 5/7 |
| Created | 2026-09-04 |
| Last reviewed | 2026-09-04 |
| Related | ADR-0006, ADR-0001, ADR-0047, ADR-0057, `SOUND-INV-016`, `SOUND-INV-021`, `P05-S006`, master plan question 7 |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

A modulation law is what a parameter's resolved value *is* when more than one thing writes
to it — an authored base, an automation lane, a note magnitude, a modulator. It is a
**delivered-behaviour** boundary: the same project with the same lanes renders differently
under two laws, and a law changed after projects rely on it changes their sound. It is also a
**product** boundary in one respect: the master plan requires that a declaration state its
law rather than pretend every parameter is a linear normalized value, and which laws exist is
a vocabulary users author against.

**Why it is ready now.** Phase 5's declaration carries, since `P05-S006`, a unit per
parameter and nothing about how writes combine. The next slice composes the parameter
layers — `stored base → automation override → modulation → mapping and clamp → smoothing →
resolved value` — and `PROCESS.md` forbids building a composition under an undecided law:
each layer's arithmetic is the law. Phase 7 then generalises modulation over the same law.

**Coupled decisions.** [ADR-0006](ADR-0006-parameter-ramp-representation.md) decides how a
value *moves* between two resolved values in time; this record decides what the resolved
value *is*. They are decided together because a ramp's endpoints are resolved values and a
law's smoothing step is a ramp. `SOUND-INV-016` fixes *when* a write takes effect and is not
reopened. `SOUND-INV-021`'s velocity destination is a write under this law, and
[ADR-0057](ADR-0057-refuse-parity-verdict-over-a-placed-note.md)'s residual — how V1's two
velocity sensitivities compose — is **Phase 6's** and is not decided here; this record supplies
the law that composition will be stated in.

## Decision boundary

It decides the **set of laws** a `ParamSpec` may declare, the **arithmetic** of each, and the
**order** of the layers. It does not decide which V1 parameter maps to which law (each native
kind declares that as it is migrated), the per-note expression vocabulary (Phase 7, carried
here by ADR-0047), smoothing durations (ADR-0006), or automation conflict resolution between
two lanes on one parameter (ADR-0012, Phase 10).

## Evidence

Ratified from real existing parameters, as master plan question 7 asks, by reading V1's own
`set_mod_offset` implementations rather than its descriptors:

- **Pitch and cutoff are semitone-additive.** `Oscillator` accumulates `pitch`, `detune`
  and `frequency` offsets into one semitone offset applied in `actual_frequency`, with a
  per-target scale; `Filter` scales a normalized offset to **±48 semitones** on `cutoff`,
  with the comment that a raw normalized product "is at most 1 semitone — inaudible". The
  law is additive in the log-frequency domain, and the *depth* is part of the target.
- **Levels and pans are additive in their own normalized or bipolar range.** `Amplifier`
  accumulates `level` and `pan` as `BipolarValue`; `Filter` accumulates `resonance` as
  `NormalizedValue`. Both clamp at the type.
- **The Mod Grid has one combine mode.** `CombineMode::Add`: `result = current + amount ×
  source`, clamped to the target's range. No multiplicative mode exists in V1.
- **A choice is not modulated.** V1's `ParamKind::Enum`, `Bool` and `Reference` parameters
  are excluded from the Mod Matrix roster; `modulatable: false` is a descriptor field.
- **Smoothing exists for level and for cutoff, per block.** `voice.rs` ramps level and
  `filter.rs` ramps the base cutoff, each across a block; the Mod Grid's `smooth` fields are
  one-pole followers for audio-tap sources rather than de-zippering. ADR-0006 owns what a
  ramp is; this record's layers end where that ramp begins.

Two things V1 does **not** have, and the master plan asks for: a decibel-additive law (V1's
level is a normalized linear value modulated additively) and a physical-linear-additive law
distinct from normalized (V1 has none with a physical unit that is modulated).

## Options

1. **One law: normalized additive with clamp** — V1's Mod Grid. Rejected: it is what the
   master plan names as pretending every parameter is a linear normalized value; pitch
   modulation under it is inaudible, which is why V1's oscillator and filter each escape
   into semitones by hand.
2. **A law per parameter, written in the kernel** — V1's `set_mod_offset`. Rejected: the
   law is then invisible to the compiler, the automation surface and discovery, and each
   module reinvents its scale (`48.0`, `DETUNE_MOD_SEMITONES`); Phase 5's declaration exists
   so that this cannot recur.
3. **A closed set of declared laws, composed centrally.** Selected. The declaration names
   the law; the parameter slot applies it in one place; a kernel reads a resolved value and
   never composes.

## Decision

1. A `ParamSpec` declares exactly one **modulation law** from a closed set, and the law is
   applied in the parameter slot — never in a kernel. The initial set, each with its
   arithmetic over a resolved `base` and a modulation sum `m`:
   - `NormalizedAdditive`: `clamp(base + m, 0, 1)`; `m` in normalized units.
   - `BipolarAdditive`: `clamp(base + m, −1, 1)`; `m` in bipolar units.
   - `SemitoneAdditive`: `base × 2^(m/12)`; `base` in hertz, `m` in semitones. Pitch and
     cutoff, as V1 already computes them.
   - `DecibelAdditive`: `base × 10^(m/20)`; `base` a linear amplitude, `m` in decibels.
   - `PhysicalLinearAdditive`: `base + m` in the parameter's physical unit, then clamped to
     the type's domain.
   - `MultiplicativeGain`: `base × m` with `m` a linear factor, identity `1`.
   - `ThresholdedBoolean`: `base + m ≥ 0.5` where the declaration explicitly supports it;
     otherwise a boolean is `NotModulatable`.
   - `NotModulatable`: a choice, a reference, or a boolean without explicit support; a write
     from any layer but the base is refused at admission, not ignored.
2. **The layers compose in the master plan's order, and every layer is optional:** the
   stored base; an absolute automation override, which *replaces* the base when present; a
   controller layer where the declaration supports one, which replaces likewise; the
   modulation sum `m` over every modulator, each `amount × source` in the law's units; the
   law's arithmetic; the type's clamp; then ADR-0006's smoothing; then the resolved value.
   A replacement layer replaces the base only, never the modulation — an automated pitch
   still bends.
3. **Modulation depth is stated in the law's units by the modulation edge, not hidden in
   the target.** V1's `±48 semitones` on a filter cutoff becomes an edge whose amount is
   `48 st × source`; a kind declares no per-target scale.
4. **A resolved value is what a kernel reads, and a kernel composes nothing.** `SOUND-INV-013`'s
   prohibition on a kernel taking a law-selecting parameter extends to this: the law is not a
   kernel input.
5. **The law is part of the declaration and so of discovery**: `node::catalog()` presents it
   beside the unit, and validation refuses an edge whose units do not match the target's law.
6. Phase 6 states velocity composition — `P04-R001` — as a modulation under one of these laws
   on the velocity destination, and inherits that residual before it does.
7. **An activation's catch-up write is an override-layer write.** `SOUND-INV-018` restores the
   last pre-destination `SetParameter` of every prepared target; the slot takes that value as
   its override and seeds ADR-0006's segment at the resolved value, and a modulator's
   contribution after the activation comes from the modulator, never from the flattened
   value. An independent read of this acceptance found the two invariants unreconciled.

## Falsifier and stopping rule

Violated if a kernel reads more than one value for a parameter, if any layer's arithmetic is
implemented outside the slot, if two native kinds compose the same law differently, if a
modulation reaches a `NotModulatable` parameter, or if a project's resolved values differ
between headless and observed renders. The set of laws is not exhaustive by intention; adding
a law is an amendment with its arithmetic stated, and is not a defect.

## Consequences and risks

- **Accepted cost.** Every V1 module's hand-written `set_mod_offset` scale becomes an edge
  amount when that module is lowered, which is a change to how a saved Mod Matrix route reads
  — Phase 7's lowering owns the mapping and the `LOWER` contract owns its diagnostics.
- **Risk: the decibel law's identity.** `m = 0 dB` is identity; a lowerer translating V1's
  normalized level offset into decibels must not map `0` to silence. Control: the law's
  identity is stated per law above and the lowering tests assert it.
- **Revisit condition.** Phase 7's first modulator that no listed law expresses.

## Specification update

Acceptance writes the law set and the layer order into the Sound Core render contract now,
as `SOUND-INV-023`, marked *not built* and owed by Phase 5's seventh slice — the form
`SOUND-INV-022` takes — so that implementation follows a current specification rather than
this record's clauses; ADR-0027's acceptance established that postponing the contract to the
building slice is not what `decisions/README.md`'s lifecycle allows. `node::catalog()` gains
the paired law beside the unit when that slice declares it. No current behaviour changes at
acceptance: nothing in V2 composes yet.

## Review

Design consultation: the three options and their costs were put to the user on 2026-09-04,
who selected option 3 together with ADR-0006's option 2.

Independent semantic reviewer: `codex review --uncommitted` over the acceptance transaction
— both records, the decision index, the Sound Core contract's two new invariants and
`NOW.md`. Its three findings and their repairs are recorded in ADR-0006's review section;
the one that reaches this record is clause 7.

Stopping rule: an arithmetic that contradicts V1's own for pitch and cutoff, a layer order
that lets modulation be replaced by automation, or a kernel that composes blocks acceptance.
