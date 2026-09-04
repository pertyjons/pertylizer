# ADR-0006: Parameter Ramp Representation

| Field | Value |
|---|---|
| ID | ADR-0006 |
| Status | Proposed |
| Phase | 5/7 |
| Created | 2026-09-04 |
| Last reviewed | 2026-09-04 |
| Related | ADR-0007, ADR-0001, ADR-0043, ADR-0046, `SOUND-INV-016`, `SOUND-INV-020`, master plan question 6 |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

How a parameter moves from one resolved value to the next is **delivered behaviour**: a
step and a ramp render different samples, and a ramp's shape is audible on every level and
frequency change a project makes. It is also a **real-time** boundary: a ramp is evaluated
per sample or per quantum inside the render loop, so its representation is bounded work the
purity scan must be able to see.

**Why it is ready now.** Master plan question 6 says to decide it "with sample-accurate
automation in Phase 3/5". Phase 3 delivered sample-positioned events (`SOUND-INV-016`,
ADR-0043's late clamp); Phase 5's next slice composes parameter layers whose last step is
"smoothing or de-zipper ramp", and an unrepresented ramp there would be a step by default —
a decision by omission.

**Coupled decision.** [ADR-0007](ADR-0007-parameter-modulation-laws.md): a ramp runs between
two *resolved* values, so the law resolves first and the ramp moves the result. Decided
together.

## Decision boundary

It decides **what a ramp is** in a compiled plan and in the renderer's state: its
representation, where its endpoints come from, and what a kernel sees. It does not decide
ramp *durations* per parameter (the declaration's smoothing policy, a `ParamSpec` field the
slot reads), nor tempo ramps (ADR-0049, a different quantity), nor automation *curves* between
lane points (a Phase 10 timeline concern that produces resolved values this ramp then moves
between).

## Evidence

- **V2 today has steps only.** A `ControlRate::Quantum` write takes effect at the next
  boundary as a step; a `ControlRate::Sample` write at its offset as a step
  (`TimedControl { offset, control, value }`). No kernel interpolates a control.
- **V1 de-zippers level per block** and nothing else: `voice.rs` ramps the amplifier level
  across a block so a level change does not click; pitch and cutoff step, and their clicks
  are what the Mod Grid's `smooth` field was added to hide, per node.
- **ADR-0001's quantum is 64 frames** (ADR-0037), so a per-quantum linear segment is 64
  samples long — long enough that a step at a quantum boundary is audible on an amplitude and
  short enough that a linear segment across it is not.
- **The purity scan must enumerate the arithmetic.** A representation that is a closure or
  a curve table would put unenumerable work in the loop; a linear segment is two numbers.

## Options

1. **Scalar only — every write is a step.** Today's state. Rejected: it makes the
   de-zipper V1 has impossible to express, and clause "smoothing or de-zipper ramp" of the
   plan a no-op.
2. **Start–end linear segment per quantum.** A control carries `(value, target, remaining
   frames)` and advances linearly; a new write sets a new target from the current value.
   Selected: it is V1's de-zipper generalised, it is bounded (three numbers, one add per
   sample), a kernel reads it as the value at each sample, and a segment across a quantum
   boundary is continuous because the state carries it.
3. **Piecewise segments with a curve.** Rejected for now: no consumer needs a shaped ramp
   between two resolved values — an automation *curve* between lane points is resolved into
   values before the slot, which then moves linearly between them — and a curve table is
   what the purity scan cannot enumerate.

## Decision

1. A parameter slot's runtime value is a **linear segment**: the current value, the target
   value, and the frames remaining; the value advances by `(target − current) / remaining`
   per frame and holds at the target. A segment with zero remaining frames is a step.
2. **Endpoints are resolved values** under ADR-0007's law; the segment moves between them
   and composes nothing.
3. **A new write retargets from the current value**, not from the previous target, so a
   write mid-ramp cannot jump; its duration is the declaration's smoothing policy for that
   parameter — `None` for a gate and any `ControlRate::Sample` destination, whose timing
   `SOUND-INV-016` owns and which must land exactly where its render position says.
4. **The representation is the same for automation, modulation and a caller's write**: the
   slot does not know who retargeted it.
5. A kernel reads **one value per sample** from its controls and never advances a segment
   itself; advancing is the slot's, in the loop, so the purity scan sees it once.

## Falsifier and stopping rule

Violated if a sample-positioned control is smoothed, if a retarget jumps from the previous
target rather than the current value, if two kernels advance the same control, or if the
loop's ramp arithmetic is anything but an add per frame. A different default duration is not
a defect.

## Consequences and risks

- **Accepted cost.** Every quantum-rate control gains three numbers of state and one add per
  frame; the resource report charges it as state bytes per parameter.
- **Risk: a ramp across a plan swap.** ADR-0009 owns crossfade; a slot's segment is part of
  the state a retirement carries, and the plan swap starts the new plan's slots at their
  resolved values. Control: the retirement crossfade already covers the discontinuity.
- **Revisit condition.** A consumer that needs a shaped ramp between resolved values.

## Specification update

Acceptance adds the slot's segment to the Sound Core render contract beside ADR-0007's law,
written by the same slice. No current behaviour changes at acceptance.

## Review

Design consultation: put to the user on 2026-09-04 with the three options; to be recorded.

Independent semantic reviewer: to be recorded at acceptance.

Stopping rule: a smoothed gate, a retarget from the wrong endpoint, or unenumerable loop
work blocks acceptance.
