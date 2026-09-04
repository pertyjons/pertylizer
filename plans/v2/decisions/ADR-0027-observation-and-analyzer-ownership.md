# ADR-0027: Observation and Analyzer Ownership

| Field | Value |
|---|---|
| ID | ADR-0027 |
| Status | Accepted |
| Phase | 0B/5/9/10E |
| Created | 2026-09-04 |
| Last reviewed | 2026-09-04 |
| Related | ADR-0021, ADR-0028, ADR-0038, `LIMIT-0020`, `LIMIT-0062`, `HOST` `max_observation_taps`, master plan question 27 |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

This is a **product choice**, and `PROCESS.md` names that as a durable-decision trigger: what a
meter, a scope, a spectrum view or an OSC listener *is* to a project — authored content, a
compiler artifact, or a host session's private state — decides what a saved project may
contain, what a headless render must reproduce, and what an external protocol may observe. It
is also a **persisted-format** boundary: if an analyzer's buffers, subscribers or connection
state were ever serialized, every later phase would inherit them.

**Why it is ready now.** The master plan requires this record `Accepted` before Phase 5
implementation begins, because Phase 5's declarative node API makes a module declaration the
single source for *observation/tap capability*. If the declaration is written before the
ownership question is answered, the answer is taken by the first declaration rather than
decided — and Phase 5's own exit gate asks that "the same project compiles headless and with
GUI/OSC observation enabled; observation changes no audible samples or semantic project
digest", which cannot be tested against an undecided ownership.

**Coupled decisions, already settled.** [ADR-0021](ADR-0021-host-profile-and-admission-policy.md)
made observation taps a `HostProfile` capacity (`max_observation_taps`, `LIMIT-0020`) whose
exhaustion is a compile error rather than a silent drop, and left to this record *what a tap is
for*. [ADR-0038](ADR-0038-engine-egress-queue-classification.md) removed V1's prioritized
event channel and its OSC-published drop counters, and left the same question here.
[ADR-0028](ADR-0028-long-running-job-contract.md) stays `Deferred` with its three standing
constraints, and whether job progress is telemetry or a delivered value is its question; this
record does not answer it. This record decides what ADR-0021 and ADR-0038 delegated, and
nothing they decided.

## Decision boundary

It decides **who owns what** across the four things that today's V1 conflates in one GUI
object: the analyzer's authored intent, the signal point it observes, the buffers a consumer
reads, and the transport an external consumer reads them through.

It does **not** decide the OSC payload's shapes or the visualizer wire format (Phase 10E
migrates them under the policy set here), the tap capacities (ADR-0021's, carried by the
`HOST` specification), the job-progress unit (ADR-0028's), or any DSP node's cost figures
(measured by the phase that builds the node). It does not change V1.

## Evidence

- **V1 conflates the four.** `EngineState` carries meter and scope buffers the GUI reads
  directly, the OSC server reads the same state, and the standalone visualizer is driven by
  OSC telemetry. V1's `prioritized_event_channel` is a separate thing: ADR-0038's inventory
  found it constructed only by its own module tests, with no in-workspace production caller,
  and warns against conflating it with the OSC note-telemetry ring — so it is not a path Phase
  10E migrates. The master plan's *Observation and external telemetry* section and its risk
  *observation and external protocols contaminate the engine* were written against the shape
  the first sentence describes.
- **The capacity side is already decided and measured.** `max_observation_taps` is `128` in
  the `HOST` specification, a V1 carry-over, with the silent drop turned into a compile error.
  Nothing here changes a number.
- **The headless path is the control.** `pertylizer render` and the corpus digests render with
  no GUI object in existence. Any ownership under which a project needs a GUI object to compile
  is falsified by that path today.

## Options

1. **Observation is host session state, as in V1.** The GUI owns buffers, injects analyzers
   into the running engine, and OSC reads engine state. Rejected: it is the shape the plan's
   risk section names, it makes headless and GUI compilation two different graphs, and a
   scope's presence can change what renders.
2. **Every analyzer is a persisted DSP node that owns its buffers.** A meter, scope or FFT
   view is authored graph content with its own runtime storage, serialized with the project.
   Rejected: buffers, frame caches and subscriber lists become project state and digest
   content; a headless render pays for views no one is watching; and a saved project carries
   session artifacts a second host cannot honour.
3. **Split ownership by kind.** Selected, and it is the master plan's own line:
   - a **persisted analyzer/monitor node** owns authored intent and parameters — what to
     observe, with what settings — and *never* a buffer, FFT frame cache, subscriber, socket
     or connection status;
   - a **compiler-declared tap** names a stable signal point with its data type, rate and
     resource cost, and is the only thing a consumer may subscribe to;
   - a **runtime subscription**, owned by the host, owns the bounded rings or atomics, the
     generation and revision tags, the decimation and the consumer's lifetime; it is admitted
     by the host against the taps the plan already holds, is lossy under a slow consumer —
     which receives drop and staleness metadata and never blocks rendering — and leaves the
     tap itself untouched: an unsubscribed tap stays in the plan unchanged, and what its
     declared cost policy may omit is the consumer-side capture and analysis, never the tap or
     the node's downstream signal;
   - **expensive analysis** — FFT, feature extraction, history accumulation, protocol encoding,
     visualization — runs on non-real-time workers unless an explicit node in the authored
     graph declares and passes a bounded real-time cost;
   - **one telemetry facade** serves GUI meters and scopes, OSC and the standalone visualizer;
     external messages carry a protocol version and the active plan or project revision, and a
     version mismatch is diagnosed rather than decoded as a nearby shape.

## Decision

1. Ownership is option 3's split, clause by clause, and the four kinds are disjoint: a value
   that belongs to one kind may not be stored, serialized or referenced through another.
2. **A persisted analyzer node serializes only authored intent.** Runtime sample buffers, FFT
   frames, subscriber lists, freeze snapshots, meter peaks and connection state are never graph
   serialization or `EditorMetadata` fields. A schema that carries one is a defect, not a
   format version.
3. **Passive observation is invisible to the render.** Adding, removing or saturating a
   subscriber changes no audio sample, no semantic project digest, and no plan: the same
   project compiles headless and with every observation enabled, to the same plan. The two
   admissions are distinct and only one can fail. The taps a plan *declares* are admitted at
   compilation against the profile's `max_observation_taps`, whether or not anyone will
   subscribe — that is a property of the project and the profile, and it fails as ADR-0021
   says, as a compile error naming the counts. A *subscription* is admitted by the host
   against the taps the compiled plan already holds; it can be refused as a subscription and
   can neither fail nor change a compilation.
4. **A tap is a compiler artifact with a declared cost, present whether or not it is read.**
   Phase 5's module declaration is the single source for a node kind's tap capability; a tap
   not declared there does not exist, a declared one exists in the plan independently of any
   subscriber, and a subscription names a tap rather than a node's internals. The tap
   capacities remain ADR-0021's and the `HOST` specification's.
5. **Workers, not the audio thread, analyse.** A real-time analysis stage exists only as an
   authored node that declares a bounded cost and passes the real-time gate every other node
   passes; a GUI cannot cause one to exist.
6. **External protocols go through the facade, versioned.** OSC and the visualizer read the
   telemetry facade and nothing else; `EngineState` access is withdrawn as their contract at
   Phase 10E's migration. The payload shapes are Phase 10E's to decide under this policy.
7. The live subscription contract — saturation, staleness metadata, consumer lifetime under a
   real host — is **verified in Phase 9**, as master plan question 27 states; this record
   decides ownership and cannot supply that evidence.

## Falsifier and stopping rule

This decision is violated if a project's serialized form or semantic digest changes when a
view is opened, closed or saturated; if a headless compilation needs, or a GUI compilation
produces, a different plan than the other; if any analyzer type serializes a buffer, frame
cache, subscriber or connection field; if a tap's presence in a plan depends on whether a
subscriber exists; if a consumer can subscribe to anything but a declared tap; or if an
external protocol reads engine state outside the facade after Phase 10E. Each is
a correctness defect and blocks the consuming slice. A tap's cost figure, a ring size or a
payload shape does not.

## Consequences and risks

- **Accepted cost.** Every existing analyzer view is rewritten as intent plus subscription;
  V1's direct `EngineState` reads for meters and scopes do not carry over. Phase 8 and Phase
  10E pay it, which is where the master plan already places the GUI and protocol migrations.
- **Safety/correctness control.** Clause 3 is testable as an equivalence: observation
  disabled, enabled and saturated must render bit-identical samples and produce one semantic
  digest. The master plan's Phase 5 exit gate carries that test in its fifth bullet — "the
  same project compiles headless and with GUI/OSC observation enabled; observation changes no
  audible samples or semantic project digest" — and it is what makes the clause more than
  policy.
- **Risk: a cost policy that "reduces" an unsubscribed tap changes the render.** Control:
  clause 4 keeps the tap in the plan and clause 3 keeps it passive; a reduction may only omit
  what is *captured for a consumer*, never the tap or what the node emits downstream. A tap
  whose absence would alter audio is not a tap but a node, and clause 5 governs it.
- **Risk: a persisted analyzer node becomes a place to smuggle session state.** Control:
  clause 2 is enforced by the same persisted-field mechanisms Phase 4 built — a register of
  every persisted name and per-type pins — so a buffer-shaped field on an analyzer type fails a
  test with the disposition question attached.
- **Revisit condition.** Phase 9's live verification, or the first consumer that needs an
  analysis stage on the audio thread that clause 5's bounded-cost path cannot admit.

## Specification update

Acceptance writes the contract into the two current specifications that implementation
follows, split where ownership splits. The Sound Core render contract gains **`SOUND-INV-022`**
— a tap is declared by a node kind, exists in the plan independently of any subscriber, is
passive toward every downstream signal, and is admitted at compilation against
`max_observation_taps`. The `HOST` specification gains **`HOST-INV-023`** — a subscription is
host-owned, bounded and lossy with drop and staleness metadata, admitted against the plan's
admitted taps, and can neither fail nor change a compilation. Both are stated now and their
conformance rows name Phase 5's first observation slice as what builds them, the same form
`SOUND-INV-021` takes for its unbuilt bend clause; an independent read of this acceptance found
the earlier draft postponing the contract to that slice, which `decisions/README.md`'s lifecycle
does not allow. Clause 6's protocol policy has no current specification to enter: the first
implementation that follows it is Phase 10E's migration, which creates the protocol's.

The `HOST` specification's ownership notes change with it: the sentence listing this record
among decisions still `Proposed`, the `max_observation_taps` row's "ADR-0027 owns what a tap
is", and the unresolved-questions row on what a tap is and who owns the analyzer surface now
cite this record. No capacity, default or admission number changes.

## Review

Design consultation: the three options and their costs were put to the user on 2026-09-04,
who selected option 3.

Independent semantic reviewers, both over the uncommitted acceptance transaction and both
of a different model family from the author. `codex review --uncommitted` found four
defects, all repaired: the contract postponed to a later slice rather than published (the
two invariants above); an unsubscribed tap allowed to be absent, which contradicted clause 3
(the tap now stays in the plan and only consumer-side capture may be omitted); a false
premise that the standalone visualizer had a prioritized event channel (it is driven by OSC
telemetry, per ADR-0038); and Phase 0B dropped from the phase list. `agy` on
`gemini-3.8-flash-high`, fed the change set inline, found eight: three restate codex's; one
found clause 3 saying observation changes "no compilability" while tap admission can fail
(the two admissions are now distinguished); one found the former clause 7 deciding job
progress that ADR-0028 owns while `Deferred` (the clause is withdrawn); one found the
subscription invariants placed in the Sound Core contract though they are the host's (the
split above); one found the `HOST` unresolved-questions row reading as closed (reworded); and
one, that Phase 5's exit gate does not carry the equivalence test, is **rejected**: the master
plan's Phase 5 exit gate states it verbatim in its fifth bullet. Its eighth finding was
consequential on the others. An earlier `agy` pass on `gemini-3.7-flash-high` found
`NOW.md` asserting Phase 5's owed obligations were listed below when they sat under
later-owned work; they are in the Phase 5 table now.

Stopping rule: an option that lets a view change a render, a serialized analyzer that carries
runtime state, or a headless plan that differs from a GUI plan blocks acceptance. Editorial
detail, a ring size, and a payload shape do not.
