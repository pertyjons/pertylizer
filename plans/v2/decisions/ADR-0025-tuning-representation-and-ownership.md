# ADR-0025: Tuning representation and ownership

| Field | Value |
|---|---|
| ID | ADR-0025 |
| Status | Accepted |
| Phase | 0B/4/6/10A |
| Created | 2026-09-02 |
| Last reviewed | 2026-09-02 |
| Related | ADR-0047, ADR-0026, `P03-R003`, `P04-R001`, `SOUND-INV-017` |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

Where a note's pitch is resolved decides who owns tuning, and that binds every phase from here.
It is a **product choice**, and the question is *not* whether Pertylizer supports non-equal
temperament — Phase 6's exit gate already requires "at least one non-12-TET/Scala mapping", and
both options below deliver it. The question is **which side of the event boundary the prepared
tuning lookup lives on**, and three things follow from the answer: whether a V2 node can behave
by key at all, whether changing a tuning is a value change or a plan swap, and whether the
master plan's rule that every pitch-producing node uses one resolved tuning context is a rule
about nodes or about producers.

**Why now.** [`P04-R001`](../master-plan.md) blocks Phase 4's exit on this record's absence.
ADR-0047 is accepted and its coupled-decision clause states that a note *pitch* field "cannot
be selected before [ADR-0025] without committing the public event contract to a pitch
representation that ADR-0025 exists to choose", so Phase 4's note payload carries neither pitch
nor velocity and no saved note may be rendered. Phase 4's exit review is `Rejected` for that
reason among others. This is not a deadline that arrived; it is a phase that stopped.

**Coupled decisions.** ADR-0026 (minimum `SampleMap` and `SampleZone` model) shares one
premise with option B below: a sample zone selects by key, so a payload with no key cannot
address one. Neither record needs the other decided first, but they must not disagree.

## Decision boundary

**The choice:** what a V2 note-on carries as its pitch, and therefore where a tuning definition
is applied — above the event boundary in the producer, or below it in the plan.

**Verified premises**, read from the tree at `2e7cd4ff`:

- The master plan already fixes the *authored* model: `TuningDefinition` is canonical authored
  data recording "scale, keyboard mapping, reference note/frequency, stable identity, and
  optional Scala/KBM source/asset provenance", and `PreparedTuning` is "the immutable
  renderer-rate lookup representation derived from it". This record does not reopen that.
- It also already states a rule this decision must not contradict: "Every pitch-producing node,
  sequencer expansion, preview path, analysis expectation, and offline/live renderer uses the
  same resolved tuning context. Direct MIDI-to-12-TET conversion is permitted only when the
  selected tuning explicitly is 12-TET." Phase 6's work list repeats it: "nodes must not
  independently hardcode MIDI-to-12-TET conversion."
- V1 has **two** pieces of key-indexed behaviour that a frequency alone cannot drive: the
  filter's `key_tracking` parameter, and `KeyboardPanner::calculate_pan`, which derives pan
  from the `MidiNote` itself. The panner is not hypothetical — `CORPUS-0006` requires its
  behaviour at feature-parity, so a V2 that cannot recover the key cannot reproduce a pinned
  corpus case.
- The sampler is **not** among them. Two drafts of this record claimed it was; an independent
  read refuted it both times. The sampler selects its audio by `sample_id`, and
  `set_voice_pitch` consumes the already-resolved `VoicePitch.played` frequency divided by the
  stored root frequency. It is frequency-driven and is evidence for neither option.
- V1 stores a note's pitch as a required `Pitch`, so **every** saved note is pitched. There is
  no unpitched saved note to render while this record is open.
- V2's node registry has no key-indexed node today. The reachability of the falsifiers below is
  therefore a question about Phases 5 and 6, not about the current tree.

**Non-goals.** How a tuning is authored, persisted, or selected in the UI; the scale format;
Scala/KBM import. Those are `TuningDefinition`'s and Phase 10A's. This record decides only what
crosses the event boundary and who resolves it.

## Options

**A. The note carries a resolved frequency; the producer applies the tuning.**
V2's event contract is post-tuning. A lowerer, a live MIDI adapter or a sequencer expansion
resolves a key through `PreparedTuning` and sends hertz; V2 never sees a key and owns no tuning.

- *For:* V2's render core stays free of a tuning model entirely, which is the smallest thing
  that can work and the easiest to reverse. Microtonal and Scala content need no V2 change —
  the producer resolves them.
- *Falsifier, and it is already reachable:* a V2 node that must behave **by key** rather than by
  frequency. Two exist in V1 — the key-tracked filter and the keyboard panner — and
  `CORPUS-0006` requires the panner's behaviour at feature-parity. Under a resolved tuning that
  is not 12-TET the key cannot be recovered from the frequency, so under this option that case
  is unreproducible without the producer sending a key beside the frequency, which is option C.
- *Falsifier, and it is a decision rather than a discovery:* the master plan's rule that every
  pitch-producing node "uses the same resolved tuning context" becomes vacuous at the node,
  because no node would have a tuning context. **Choosing A therefore includes amending that
  rule** to state it over producers. That amendment is part of the choice and not a
  consequence to be discovered afterwards.

**B. The note carries a key; the plan carries a `PreparedTuning` that resolves it.**
The event contract is pre-tuning. A note-on names a key identity, the compiled plan holds the
immutable prepared tuning the master plan already describes, and every pitch-producing node
resolves through it.

- *For:* it is what the master plan's own wording already implies, and it is the only option
  under which "nodes must not independently hardcode MIDI-to-12-TET conversion" is a rule about
  nodes. Both key-indexed behaviours work, and ADR-0026's sample zones need no second channel.
- *Falsifier:* the tuning becomes part of the plan, so changing it is a plan swap. **The
  observable condition:** if a supported interaction requires the tuning to change while a note
  is sounding — a per-note retune, a tuning automated over a bar — this option requires a plan
  swap per change, and Phase 9's activation cost per swap is what says whether that is
  affordable. If no such interaction is supported, the condition is never met and this
  falsifier never fires.
- *Falsifier:* the render core acquires a lookup on the audio path, with its own admission cost
  and determinism obligation. **The observable condition:** the lookup's per-note cost measured
  against the host profile's admission budget, on the same instrument the Phase 2 evidence used.
  Under it, this is a value read; over it, the option is refused on its own numbers.

**C. Both: a key identity and the frequency the producer resolved.**
The note carries the key for key-indexed behaviour and the resolved frequency for oscillators.

- *For:* every consumer gets what it needs with no lookup on the audio path. And they are not
  redundant, which two drafts of this record got wrong: under a non-12-TET or transposed
  mapping a key and its resolved frequency describe **different dimensions**. The key is what
  the user played and is what drives zones, panning and key tracking; the frequency is what the
  tuning resolved it to. A key of C4 resolving to a C#4-equivalent frequency is not a
  contradiction — it is what a transposed mapping *means*.
- *Falsifier:* nothing in the type system keeps the pair consistent, so a producer that resolves
  a key against one tuning and stamps a frequency from another emits an event both readings of
  which are individually sensible. That is weaker than the case ADR-0047 clause 1 removed —
  there, two fields named the *same* thing — but it is a real obligation on every producer, and
  the record accepting C must say where it is enforced.

**Per-note bend composition is this record's, and each option must answer it.** The decision
register puts "per-note bend composition" inside ADR-0025's scope, to be decided before Phase 6
— so it cannot be deferred to Phase 6 as an earlier draft did. ADR-0047 clause 9 reserves the
event that carries a bend; what a bend *value means* is open, and the options answer it
differently:

- Under **A**, a bend is a resolved frequency, and its meaning is settled: the producer applies
  the tuning to the bent key and sends hertz. This is A's strongest point.
- Under **B**, "a key offset" is not yet implementable. Cents, conventional semitones, and
  scale degrees give different pitches under a non-equal mapping, and a fractional scale degree
  needs an interpolation rule for keys the mapping does not define. **Choosing B includes
  choosing among those**, and this record must state which before it is accepted.
- Under **C**, the same question arises for the key limb, with A's answer available for the
  frequency limb.

**Status quo.** No pitch at all. Phase 4 cannot render a saved note, Phase 5 cannot migrate a
key-tracked module, and Phase 6 cannot begin. It is not a viable resting place; it is where the
work stopped.

## Evidence

- `crates/synth_modules/src/filter.rs` — `key_tracking`, a key-indexed parameter.
- `crates/synth_modules/src/sampler.rs` — root-note-relative playback speed via
  `MidiNote::to_frequency`.
- `crates/synth_sequencer/src/note.rs` — `pub pitch: Pitch`, required on every saved note.
- The master plan's *Tuning and sample mapping* section and Phase 6 work list, quoted above.
- [ADR-0047](ADR-0047-note-identity-in-the-event-contract.md)'s coupled-decision clause, which
  is why this record blocks a phase, and its velocity limb, which is why velocity is not
  tuning-coupled.
- [ADR-0051](ADR-0051-locate-catch-up-gate-exception.md)'s verified-in-code section, which
  records that "a gate is **boolean and edge-triggered**" and that `NoteEdge::value` is `ONE`
  or `ZERO`. That is what forbids a velocity-valued gate.
- **V1's velocity law**, read at `d01d0322` and recorded here because no inventory row owns it
  yet: `crates/synth_modules/src/envelope.rs:257` computes
  `velocity_sensitivity(velocity, sensitivity)` and `:375` multiplies the **completed** envelope
  level by it, so the attack target stays `1.0` and the authored sustain stays the internal
  target; `crates/synth_engine/src/voice.rs:1198` then applies an independent
  `velocity_to_amp` scale to the voice's output. Those two sites are the two destinations the
  clause above names.
- **V2's envelope handoff**, `crates/synth_engine_v2/src/node/kernels.rs:1043`, which assigns
  `level = 1.0` unconditionally when the attack completes. It is why aiming the attack at a
  velocity does not work even in isolation.

**Uncertainty that could change the decision.** How many key-indexed nodes V2 will end up with.
One exists in V1 today — the key-tracked filter — and ADR-0026's sample zones would be the
second. If the answer is "those two and no more", option A's redundant-second-number problem is
small and bounded; if key-indexed behaviour turns out to be common, option B is the shape that
carries it. **That is a product question about how much of V1's sound design V2 must
reproduce, and this record cannot answer it from the code.**

## Velocity, which travels with pitch

`P03-R003` and `P04-R001` name pitch **and velocity**, so acceptance of this record cannot
close either unless velocity is settled with it. It is, and it is not coupled to tuning:

- The on edge carries **one** validated normalized magnitude. V1 consumes one saved velocity
  twice — the envelope scales by its own sensitivity and voice output scales by
  `velocity_to_amp` — and two consumers of one fact do not need two payload values.
- It is not `synth_core::Velocity`, whose constructor silently clamps; the V2 boundary refuses
  an out-of-range value rather than replacing it.
- V2 assigns it no dynamics curve. How the two sensitivities compose is Phase 6's composition
  law, exactly as tuning's authored model is Phase 10A's.
- The release edge carries no velocity. Release velocity is Phase 6's expression model.

**This is an acceptance condition, not a consequence.** Accepting this record means accepting
the velocity clause above with the pitch option selected.

### Why velocity cannot be built ahead of this record

ADR-0047 says velocity carries no tuning coupling, and that reads like an invitation to build
it while the pitch options wait. It was attempted as a design frame and refused, for four
reasons found in the tree rather than argued:

- **The gate cannot carry it.** The cheap shape would be for the on edge's existing gate value
  to *be* the velocity, since the envelope kernel already receives a `ParameterValue` and
  discards its magnitude. But [ADR-0051](ADR-0051-locate-catch-up-gate-exception.md) records as
  a verified fact that "a gate is **boolean and edge-triggered**" and that `NoteEdge::value` is
  `ONE` or `ZERO`. Making `0.5` and `1.0` audibly different would change every gate automation
  in the engine, not only note-ons.
- **So velocity needs the same expansion pitch needs.** Without the gate, a velocity is a
  second control write from one note event — the fan-out that changes `DueEvent`'s one resolved
  target, the timed-control scratch relation and `SOUND-INV-016`'s one-control wording. That
  work is identical for pitch, so doing it for velocity alone buys nothing and would be
  re-reviewed when pitch arrives.
- **V1's law is not the shape a naive port assumes.** V1 does not attack to the velocity. It
  attacks to `1.0`, keeps the authored sustain as its internal target, and multiplies the
  **whole** emitted envelope by `1 − sensitivity × (1 − velocity)` — then voice output applies
  an independent `velocity_to_amp` on top. A V2 envelope whose attack aims at the velocity
  hard-codes full sensitivity and drops the authored control; and it breaks internally, because
  the attack handoff assigns `level = 1.0` unconditionally, so a velocity-0.5 attack would jump
  to full scale at the handoff and an instantaneous attack would discard velocity entirely.
- **One value, but two destinations.** The clause above is unchanged: the payload carries one
  magnitude, and two consumers of one fact do not need two fields. What the investigation
  confirmed is narrower and is about *destinations*, not values — V1 applies that one magnitude
  at the envelope and again at voice output, so a lowering that routes it to a single control
  reproduces neither. That is the master plan's reason, and it survives.

Velocity therefore lands with pitch, under this record, and not before it.

## Decision

**Select option B.** The user chose it on 2026-09-02, on the reasoning below.

**The event contract is pre-tuning.** A note-on names a validated **key identity** — a keyboard
position, not a frequency and not a scale degree. No node converts a key to a frequency on its
own.

**A prepared tuning is referenced per node, shared per scope.** Phase 10A's exit gate requires
"project/default and per-instance tuning references", so a plan holding one tuning would make
two instruments resolve through the same scale — which this record said it would not reopen and
would have reopened. Each pitch-producing node instead references a prepared tuning; every node
of one execution scope references the same one; the table is shared rather than copied, charged
once to the plan's immutable total with the reference charged per node; and derivation is
deterministic and digested. `SOUND-INV-021` states all of it.

**How a key reaches a node that is not the played node is part of this decision, not a
consequence of it.** Choosing B does not dissolve the structural fact that in the smallest real
voice the gate belongs to an envelope and the pitch to an oscillator. `SOUND-INV-021` resolves
it by **execution scope**: admission collects the pitch and velocity destinations declared by
node kinds within the played node's scope, so the producer still names only a node. A plan in
which **one execution scope holds two playable nodes** is refused — `ExecutionScope::Voice` is a
kind rather than an instance, so two instruments' nodes in it are indistinguishable and a note
for one would reach both — while distinct scopes are allowed; Phase 6 supplies instance identity
and generalises it. *(Corrected 2026-09-04 under the factual-claim rule: an earlier wording said
"more than one voice scope", which a single scope kind cannot express and which neither the
sound contract's rule nor the implementation states.)*

**A prepared tuning is total over the key range**, so the renderer never meets a key it cannot
resolve — a node that could not has no safe answer on the audio thread, since it can neither
allocate a diagnostic nor pick a frequency. Totality is structural in the table type.

Preparation additionally refuses an entry that is not a usable frequency. It does **not**
establish that every entry was authored: an implementation read found that the table type
carries no record of which keys a definition mapped and extrapolates the rest, so a partial
mapping cannot be detected there. Completing a partial definition is the authored model's job
and Phase 10A's, and `SOUND-INV-021` says so rather than claiming a refusal that is not
possible.

**A per-note bend is a continuous offset in cents, applied after the key is resolved.** This
limb had to be settled with the option, because the decision register puts per-note bend
composition inside this record's scope and "a key offset" is not implementable without it:

- **Cents** is selected. The tuning owns *which frequency a key is*; a bend owns *how far the
  note has been pulled from it*. Keeping them separate means a bend needs no knowledge of the
  scale, and a scale needs no knowledge of bending.
- **Semitones** is *not* refused on technical grounds, and two drafts of this record claimed it
  was. Independent reads corrected both: `synth_core` defines 100 cents as exactly one semitone
  under the same logarithmic ratio, so a continuous semitone offset assumes no more about the
  scale than cents does; and Scala files are not cents-only — this repository's own parser
  accepts ratios beside them. Cents is therefore a **product choice, not a technical one**:
  it is the finer unit, it is what tuning tables are conventionally discussed in, and a
  cent-valued bend reads the same way beside a cent-valued detune. Semitones would work.
- **Scale degrees** is refused, and this one is technical: it needs an interpolation rule for
  fractional degrees, and it is not what the hardware produces — MPE and MIDI pitch bend are
  continuous deviations, not steps.

**The velocity clause above is accepted with this**, as its own text requires.

**What this does not decide.** How a tuning is authored, persisted or selected, which is
`TuningDefinition`'s and Phase 10A's; the composition law that combines a bend with vibrato,
glide and channel expression, which is Phase 6's; and the exact type of the key identity, which
the specification update below fixes.

### Why B rather than A or C

The deciding question was which option gets *more expensive as the engine grows*.

- Under **A**, every feature that reads the key rather than the frequency is a contract change.
  Two such features exist in V1 already — the filter's key tracking and `KeyboardPanner`, whose
  behaviour `CORPUS-0006` requires at feature-parity — and ADR-0026's sample zones would be the
  third. That cost has no ceiling.
- Under **C**, every *producer* carries a standing obligation to keep two values consistent,
  with nothing enforcing it. Producers are few, so the cost is bounded — but it is the kind that
  surfaces as a bug years later. ADR-0047 clause 1 faced the same shape on the release edge and
  removed the redundant field rather than requiring agreement; C is that shape again.
- Under **B**, the cost is a plan rebuild when the tuning changes, and a lookup in the render
  core. A tuning is chosen once per project and almost never mid-playback, and the rebuild is
  machinery Phase 9 builds regardless. Detune — the thing users actually adjust constantly — is
  an offset on top and is unaffected.

B is also the only option under which the master plan's existing rule, that nodes "must not
independently hardcode MIDI-to-12-TET conversion", is a rule about nodes. It places a concept
the plan already commits to rather than adding one.

## Consequences and risks

- **Accepted cost:** a tuning change rebuilds the plan, and each pitch-producing node carries a
  prepared tuning reference that admission charges for. A tuning is chosen once per project and
  almost never mid-playback; detune, which users adjust constantly, is an offset on top and is
  unaffected.
- **Safety/correctness control:** a prepared tuning is **total** over the key range, so the
  audio thread cannot meet a key it cannot resolve, and preparation refuses any entry that is
  not a usable frequency. It does **not** refuse a partial mapping — an earlier draft of this
  bullet said it did, contradicting this record's own body and the specification, and an
  independent read caught it. Until the payload is built, `smoke_render` still refuses a saved
  note and names `P04-R001`, so the absence cannot be mistaken for a faithful render.
- **What acceptance does not do:** it **unblocks** the payload work and closes nothing.
  `EventPayload::Note` still carries only an identity and an edge, so `P03-R003` and `P04-R001`
  stay open until the typed fields, the event-cardinality expansion, the lowering, the render
  and the tests land against the invariant below. `PROCESS.md` is explicit that classifying a
  decision is not a phase outcome, and this record is not an exception.
- **Revisit condition:** a caller that must retune while a note sounds, which this record's
  plan-rebuild cost would make expensive; or a second tuning consumer that is not a node, which
  per-node preparation would not reach.

## Specification update

`SOUND-INV-017`'s closing paragraph — which today records that the invariant "does not fix
pitch or velocity" — is replaced, and the render contract gains a new invariant for the note
payload. That invariant must fix, at minimum:

- **The key identity's type.** A keyboard position in `0..=127`, validated at the boundary
  rather than clamped: `synth_core::MidiNote::new` clamps a value above 127 to 127, which is
  the silent replacement of persisted input `AGENTS.md` forbids at a domain boundary.
- **The velocity's type**, per the velocity clause above.
- **How a key reaches a node.** `SOUND-INV-016` says a note-on names a node and the node kind
  owns which control it moves. A key and a velocity are two magnitudes beside the gate, so this
  is the event-cardinality expansion both this record and `P04-R001` name, and it changes
  `DueEvent`'s one resolved target and the timed-control scratch relation. That accounting is
  part of the implementation, not a consequence to be discovered.
- **Where `PreparedTuning` lives in the plan**, what admission charges for it, and that two
  renders of one plan resolve identically.

## Review

Reviewer: Codex, on the uncommitted change, across five rounds. Before selection it refused the
frame twice — the first framing asked about non-12-TET support, which does not distinguish the
options; two drafts cited the sampler as key-indexed evidence, which it is not; option C was
rejected as self-contradictory, which it is not; and per-note bend was deferred to Phase 6,
which the decision register puts inside this record.

On the selection itself it found that a single plan-wide tuning would break Phase 10A's
per-instance requirement, that an unmapped key had no defined behaviour, that the
cents-versus-semitones rationale was technically false, and that the record could not be marked
`Accepted` while the render contract still said pitch was unfixed. All four are repaired above
and in `SOUND-INV-021`.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract,
safety/correctness defect, or evidence incapable of supporting the claim. Editorial detail does
not block.
