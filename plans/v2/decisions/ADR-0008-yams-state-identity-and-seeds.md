# ADR-0008: YAMS State Identity and Seeds

| Field | Value |
|---|---|
| ID | ADR-0008 |
| Status | Accepted |
| Phase | 7 |
| Created | 2026-09-09 |
| Last reviewed | 2026-09-09 |
| Related | ADR-0004, ADR-0007, ADR-0009, ADR-0010, P06-R001, P07-S005 |
| Supersedes | — |
| Superseded by | — |

## Boundary and readiness

The first V2 script state binds subsequent plan recompilation and seed determinism.
P07-S005 needs its identity now. Live program replacement depends on Phase 9's
activation policy and is not accepted by this record's Phase 7 decision.

## Decision

1. A project supplies a typed seed explicitly when installing a script program in
   the experimental IR. Script nodes have the same stable NodeId as native nodes.
2. A script's local parameter key is its authored identifier. The node's retained
   authoring identity assigns a checked, monotonically increasing ParameterId and
   keeps removed names reserved. Source edits are compiled against a staged clone;
   a failure commits no identity changes. A renamed parameter gets a new ID and a
   removed key has no active slot, so an existing lane is refused rather than
   retargeted. Source order is never identity across edits.
3. Mutable VM state belongs to one node and one voice instance. Its seed derives
   from project seed, node identity, stable voice index, and an explicit script-state
   identity. It never includes program order, buffer slot, plan revision or table
   identity. Voice index is the runtime voice's declared identity, not a note key or
   occurrence identity. An explicit renderer voice reset restarts that voice's
   deterministic stream; an ordinary note-on does not itself reset a free-running
   Control program.
4. Named state cells and compiler-assigned stateful operations have a recorded
   layout signature. No state transfers between distinct layouts in Phase 7.
   An activation starts a new deterministic runtime state; it does not pretend to
   reconstruct the random history omitted by a seek.
5. Control state advances exactly once per internal quantum, including a silent
   voice, and resets at the first quantum boundary at or after a voice reset.
   Audio state advances per sample and resets at the sample-positioned reset.
6. The compiler, resource binding, allocation and source-name resolution run
   off-thread. The renderer borrows immutable bytecode and fixed register storage.
7. The reload half stays Deferred to Phase 9: the live host must define compatible
   program-only replacement, compile failure retaining the old program, interface
   recompilation, and reset/crossfade for incompatible state before enabling reload.

## Falsifier and stopping rule

Identical event streams and seeds must produce identical bits across host partitions,
repeated compilations and graph declaration order. Changing one voice's state cannot
change another's random sequence. A renamed or removed local parameter must fail
binding instead of writing another parameter. Violation blocks acceptance.
