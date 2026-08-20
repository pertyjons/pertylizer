# Core V2: Current Work

Last updated: 2026-08-20

This is the only authority for active Core V2 task state, blockers, and next actions. Durable reasoning and measurements
live in the linked ADRs, specs, and EVDs rather than being repeated here.

## Primary stream: Phase 2

Outcome: render one complete compiled monophonic voice through validated, scheduled V2 nodes and close Phase 2's
equivalence, CPU, and quantum gates.

### Active slice

**P02-T008 and P02-T009 are collected.** Both records are `Complete`; what remains of the slice is
[ADR-0042](decisions/ADR-0042-envelope-segment-shape.md)'s acceptance and the one manifest entry it names.

- **P02-T009** — [EVD-0014](evidence/phase-02/EVD-0014-minimal-patch-cpu.md), **Supported by rule 1**. V2 costs
  **78.0% less** than V1 on the governing pair under the conservative variant, at 189 times the noise floor. The exit
  gate's fifth bullet closes with **no margin claimed**.
- **P02-T008** — [EVD-0013](evidence/phase-02/EVD-0013-minimal-patch-equivalence.md), **Not supported**, and the
  reason is narrow. Five of its six declared thresholds pass, four by one to two orders of magnitude; E2a is exceeded
  on `fall_to_50_ms`, and its own falsifier turns on any declared threshold being exceeded. **No CORPUS-0001 claim is
  broken** — P2 claims landmark parity and V2 delivers it — so ADR-0040 clause 4's failure branch does not apply and
  the gate's third bullet is not blocked by this. E2a turned out to bound a field that is not a landmark, which is a
  defect in the record rather than in either engine, and is recorded rather than repaired by moving the number.

Blockers: none. What is open:

1. **Accept ADR-0042**, which needs a reader who did not author it, and then add `CORPUS-0001-C2` to
   `corpus/v2-reference/manifest.json`. No `preserve` claim is edited and no fixture digest moves.
2. Until then the envelope's segment-shape difference — **+1.137 dB** in the release, measured from each engine's own
   gate — has no named disposition. It is the only thing in either record still open.

Three things this slice established that are worth not rediscovering:

- **The V1 chain applies an equal-power centre pan three times**, not two: the amplifier, the stereo output, and the
  instrument fader in `SynthEngine::process`. Control C2 found the third by failing at +3.008 dB.
- **The two engines' filters are the same recurrence**, and E3a measured their magnitude responses agreeing to
  +0.068 dB across six octave bands. The sines are not the same, and neither are the envelopes.
- **A contract about the dependency graph belongs to the resolver.** The `crate_boundary` check was defeated by four
  successive valid TOML spellings before it stopped scanning manifests and started asking `cargo tree`.

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
| P02-T006          | Ready to close as not-happening  | ADR-0040 clause 5     |
| P02-T012          | Complete                         | EVD-0010              |
| P02-T013          | Complete                         | EVD-0011              |
| P02-T007          | Complete                         | `note_events`         |
| P02-T010          | Complete                         | EVD-0012              |
| P02-T008/P02-T009 | Complete — evidence collected    | ADR-0042 acceptance   |
| P02-T011          | Waiting                          | All Phase 2 outcomes  |

### Next actions

1. Accept ADR-0042 and add `CORPUS-0001-C2` to the corpus manifest.
2. Close P02-T006 as not-happening and record the dropped extraction as a deviation, per ADR-0040 clause 5.
3. Close `render_loop_purity`'s provenance gap for SOUND-INV-013: it proves every *registered* kernel is
   defined in the checked region, but not that a descriptor's function pointer resolves inside it.
4. Audit the specification's pre-existing conformance rows, which predate this phase: an independent read
   found four that name a check not carrying the invariant — SOUND-INV-001/004, 003/012, 007 and 008. Not a
   defect in the code, and recorded in the specification's unresolved questions.
5. Decide where a note's **pitch and velocity** live. P02-T007 deliberately gave `NoteEdge` neither, because
   nothing in Phase 2 reads either; Phase 3's ingress is the first task that has to.
6. Complete the Phase 2 exit review.

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
