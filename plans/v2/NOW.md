# Core V2: Current Work

Last updated: 2026-08-20

This is the only authority for active Core V2 task state, blockers, and next actions. Durable reasoning and measurements
live in the linked ADRs, specs, and EVDs rather than being repeated here.

## Primary stream: Phase 2

Outcome: render one complete compiled monophonic voice through validated, scheduled V2 nodes and close Phase 2's
equivalence, CPU, and quantum gates.

### Active slice

**P02-T008 and P02-T009 are closed.** Both evidence records are `Complete`,
[ADR-0042](decisions/ADR-0042-envelope-segment-shape.md) is `Accepted`, and `CORPUS-0001-C2` is in the manifest.
The slice has nothing open.

- **P02-T009** — [EVD-0014](evidence/phase-02/EVD-0014-minimal-patch-cpu.md), **Supported by rule 1**. V2 costs
  **78.0% less** than V1 on the governing pair under the conservative variant, at 189 times the noise floor. The exit
  gate's fifth bullet closes with **no margin claimed**.
- **P02-T008** — [EVD-0013](evidence/phase-02/EVD-0013-minimal-patch-equivalence.md), **Not supported**, and that is a
  verdict on its own thresholds rather than on the gate. Five of six pass, four by one to two orders of magnitude; E2a
  is exceeded on `fall_to_50_ms`, and its falsifier turns on any declared threshold being exceeded. **No CORPUS-0001
  claim is broken** — P2 claims landmark parity and V2 delivers it — and **every difference now carries a
  disposition**, so the gate's third bullet takes its second branch: a documented intentional difference.
- **ADR-0042** — V2's linear envelope segments are intentional. `CORPUS-0001-P2` is untouched; the shape is
  `CORPUS-0001-C2`, at **+1.137 dB** in the release measured from each engine's own gate. No fixture digest moved.

Three things this slice established that are worth not rediscovering:

- **The V1 chain applies an equal-power centre pan three times**, not two: the amplifier, the stereo output, and the
  instrument fader in `SynthEngine::process` — the third outside the voice's module graph. Control C2 found it by
  failing at +3.008 dB.
- **The two engines' filters are the same recurrence**, and E3a measured their magnitude responses agreeing to
  +0.068 dB across six octave bands. The sines are not the same, and neither are the envelopes.
- **A contract about the dependency graph belongs to the resolver.** `crate_boundary` was defeated by five successive
  valid TOML spellings before it stopped scanning manifests and started asking `cargo tree`; `--target all` is
  required, or host-target resolution hides a `cfg(windows)` entry.

References:

- [EVD-0012: what the render quantum costs on the real V2 path](evidence/phase-02/EVD-0012-render-quantum-real-path.md),
  whose estimator both records reuse
- [ADR-0040: V2 owns its DSP](decisions/ADR-0040-v2-owns-its-dsp.md), clause 4 for the disposition rule
- [Current Sound Core render contract](specs/spec-sound-core-render-contract.md)
- [Historical Phase 2 execution record](phases/phase-02-minimal-compiled-voice-graph.md)

### Phase 2 task state

| Task              | State                            | Next dependency       |
|-------------------|----------------------------------|-----------------------|
| P02-T001–T005     | Complete                         | —                     |
| P02-T006          | **Closed — not happening**       | REV-P02 registers it  |
| P02-T012          | Complete                         | EVD-0010              |
| P02-T013          | Complete                         | EVD-0011              |
| P02-T007          | Complete                         | `note_events`         |
| P02-T010          | Complete                         | EVD-0012              |
| P02-T008/P02-T009 | Complete                         | —                     |
| P02-T011          | Waiting                          | All Phase 2 outcomes  |

### P02-T006 is closed, and what that leaves

**The kernel extraction into `synth_dsp` is not happening**, and it is not deferred.
[ADR-0040](decisions/ADR-0040-v2-owns-its-dsp.md) clause 5 is the authority and the closure adds nothing to it: the
task carried no code, so closing it is a change of state rather than of the tree. The eleventh corpus fixture went
with it, and clause 5 makes no claim on it either way — the manifest's coverage is P00A-T001's subject. Confirmed
against the manifest: ten cases, and its one `planned` entry is Phase 0B's sampler category, unrelated.

`synth_engine_v2` depends on `synth_core` and `thiserror` and on nothing else, so there is no residue of the
extraction in the crate either.

**The task's state is closed; its deviation is not yet registered, and those are two different things.** Dropping a
master-plan work-list item is a deviation, and the place it belongs is `REV-P02`'s *Deviations and residual risks*
table — which does not exist yet, because writing it is P02-T011. So the registration is **owed**, with ADR-0040
clause 5 as the acceptance basis it will carry, and the frozen phase record's own deviation list is the other input
that review has to fold in.

### Next actions

**`P02-T011` is the only Phase 2 task left that is neither complete nor closed**, and the four items before it are
its inputs rather than separate work.

1. Close `render_loop_purity`'s provenance gap for SOUND-INV-013: it proves every *registered* kernel is
   defined in the checked region, but not that a descriptor's function pointer resolves inside it.
2. Audit the specification's pre-existing conformance rows, which predate this phase: an independent read
   found four that name a check not carrying the invariant — SOUND-INV-001/004, 003/012, 007 and 008. Not a
   defect in the code, and recorded in the specification's unresolved questions.
3. Decide where a note's **pitch and velocity** live. P02-T007 deliberately gave `NoteEdge` neither, because
   nothing in Phase 2 reads either; Phase 3's ingress is the first task that has to.
4. Complete the Phase 2 exit review, `REV-P02`. Two deviation inputs are waiting for it: **P02-T006's dropped
   extraction**, whose acceptance basis is ADR-0040 clause 5, and the six deviations the frozen phase record lists.

## Paused parallel stream: Phase 0B

Outcome: complete the V1 migration inventories and the durable Project and Application Core contracts required before
Phase 10.

This phase remains active in the roadmap, but its execution stream is paused
while the Phase 2 slice above is active. On resumption, select exactly one task,
copy its observable completion check here, and only then mark it `Active`.

| Task           | State       | Resume boundary                                                            |
|----------------|-------------|----------------------------------------------------------------------------|
| P00B-T001      | Paused      | Assign evidenced dispositions in the state-ownership inventory             |
| P00B-T002      | Paused      | Assign reachability and migration dispositions in the capability inventory |
| P00B-T003      | Paused      | Resolve the two format questions that block ADR-0014 review                |
| P00B-T004–T009 | Not started | Follow the decomposition in the frozen execution record                    |

This stream does not block Phase 2. Its detailed audit chronology remains in
the [historical Phase 0B execution record](phases/phase-00b-inventories-and-project-contracts.md); new operational state
is recorded only here.

Phase lifecycle and completed gates are recorded once in
[`ROADMAP.md`](ROADMAP.md#phase-order).

## Later owned work

- Phase 3 owns renderer ingress, deferred-event storage, event scheduling, and the pending ADR-0001 clarification.
- Phase 4 owns current-project lowering and the long-running job contract.
- Phase 0B gates Phase 10.
- ADR-0039 and `LIMIT-0017` remain Phase 10E work.
