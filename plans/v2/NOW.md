# Core V2: Current Work

Last updated: 2026-08-26

This is the only authority for active Core V2 task state, blockers, and next actions. Durable reasoning and measurements
live in the linked ADRs, specs, and EVDs rather than being repeated here.

## Phase 2 is closed

**[`REV-P02`](reviews/phase-02-exit-review.md) is `Accepted`**, so Phase 2's state in
[`ROADMAP.md`](ROADMAP.md#phase-order) is `Complete` and every P02 task is closed. All six master-plan gate bullets
close. The review carries the gate table and the figures; only the consequences are stated here.

- `Q` = 64 is **confirmed** ([EVD-0012](evidence/phase-02/EVD-0012-render-quantum-real-path.md)), so the restriction on
  hand-unrolled kernels, `Q`-specific buffer layouts and tests asserting a control rate in hertz **is discharged**.
- Equivalence takes the gate's **second branch** — a documented intentional difference. **No `CORPUS-0001` preserve
  claim is broken** ([EVD-0013](evidence/phase-02/EVD-0013-minimal-patch-equivalence.md)); the envelope shape is
  `CORPUS-0001-C2` under [ADR-0042](decisions/ADR-0042-envelope-segment-shape.md).
- CPU closes with **no adapter margin claimed**
  ([EVD-0014](evidence/phase-02/EVD-0014-minimal-patch-cpu.md)).
- P02-T006's dropped `synth_dsp` extraction is **registered** in the review's deviation table, with ADR-0040 clause 5 as
  its acceptance basis. That debt is paid.

The review's *Deviations and residual risks* table is the register for everything Phase 2 leaves behind; it is not
repeated here. Two items are owed to later work rather than to nobody, and both are named there: **a note's pitch and
velocity** (Phase 3's ingress is the first consumer — `NoteEdge` carries neither today), and **`SOUND-INV-012`'s second
sentence**, the one invariant with no executable check.

## Active stream: Phase 3 sample-accurate scheduling

Phase 3 is `Active`. It was activated on 2026-08-25 after Phase 2 closed and the maintainer corrected the phase
boundary: Phase 3 consumes events that are already expressed as the current epoch's `SampleTime`; it does not need a
physical device-clock mapping to schedule them. [ADR-0022](decisions/ADR-0022-hardware-time-mapping.md) remains
`Deferred`, but its retained release-platform, adapter-clock, arrival-fallback, and replacement-mapping evidence now
gates Phase 9 exit and any qualified live-timing claim rather than Phase 3 entry.

### Completed bounded slice — compiled note across host partitions

The first implementation slice had one observable completion boundary:

- a compiled-event scheduler presents only events belonging to quanta that the next actual renderer call will produce,
  rejects an epoch mismatch or a schedule that would present an event late, and performs no real-time allocation;
- one note with non-quantum-aligned on and off positions is rendered through actual host-call families `1 x 4096`,
  `16 x 256`, `64 x 64`, and a predeclared irregular partition of the same 4,096 frames;
- all four outputs are bit-identical, and both edges occur at the requested `SampleTime` plus the renderer's declared
  constant `Q` live-output carry, with no late-event diagnostic.

This slice does not add live or arrival-time ingress, a CPAL or MIDI mapping, tempo/session ordering, or final producer
share and capacity values. Simulated timestamped ingress equivalence remains Phase 3 exit work: the same `SampleTime`
sequence presented through a deterministic simulated-ingress producer must equal the precompiled sequence. Physical
hardware equivalence is deliberately not claimed by that test.

That boundary is met by the compiled scheduler in
[`schedule.rs`](../../crates/synth_engine_v2/src/schedule.rs) and its actual-callback conformance test in
[`compiled_schedule.rs`](../../crates/synth_engine_v2/tests/compiled_schedule.rs). The next Phase 3 implementation
slice is deliberately not selected by this completion update.

### Completed slice — ADR-0046's producer shares

`beddf91b` adds the seven ground-2 profile fields ADR-0046 clause 1 creates, with the plan-independent relations
profile construction can decide: the share sum against `max_events_per_quantum`, positivity per field,
`release_event_share >= release_hold_capacity`, and `max_scheduled_events_in_flight` against `compiled_event_share`
over a derived `max_quanta_per_callback`. `QuantumCount` exists so that derived value carries its unit.

Two consequences of the partition are load-bearing and were found by existing tests rather than predicted: six
positive shares cannot fit a cap below six, so a `max_events_per_quantum` of 1 or 4 is no longer representable; and
the compiled floor makes a very small `max_scheduled_events_in_flight` unconstructible.

The defaults are provisional. Later slices closed two of the three obligations this one left: the ingress store was
registered and the live share's lower bound implemented against it, and the arbiter's preparation now covers the
sealed-batch extent. **One** obligation stays open, named in the host-profile specification's deferred list rather
than repeated here: the measurement that must reselect the partition, the cap and the ingress depth before live
ingress.

### Completed slice — the publication arbiter's sealed batch and share ledger

The first arbiter slice builds ADR-0046 clause 2's store and clause 1's ledger, and nothing else: producers, ingress
reads, scheduler evaluation and the renderer wiring are not in it.

- The store is preallocated to `max_events_per_quantum * max_quanta_per_callback` and written **by index**, never
  grown. The real-time rules forbid `Vec::push` even where capacity usually happens to be available, and the purity
  scan caught exactly that in the first draft.
- Every event is charged to exactly one of six producer classes, per destination quantum. A class overrunning its own
  share is a fault **even while the quantum total has room** — clause 7's rule that slack is not recovery capacity —
  and that is what the ledger is keyed on.
- Sealing is a type, not a flag: `Publication::seal` consumes the writer and returns a read-only `SealedBatch`, so a
  write after sealing does not compile.
- High-water occupancy per class survives `open`, because it describes the stream rather than the call, and it is what
  the outstanding measurement will read. A per-quantum **external** total is kept beside the six class marks, because
  those peaks can fall in different quanta so their sum overstates and their maximum understates. It is named external
  rather than total: `HOST-INV-021`'s total ledger also counts the renderer-internal arena, which this slice does not
  build, and calling a partial figure the total would understate occupancy by exactly the internal share.

`src/publish/hot.rs` joins the real-time purity scan's region. Three properties are mutation-verified: refusing on the
share rather than the quantum total, the high-water mark surviving a quieter pass, and per-quantum rather than
per-call accounting.

**A fault is reported and not yet enacted.** Clause 7's terminal renderer response — silence over this callback and
every later one in the epoch, both carries invalidated, `needs_reprepare` published — belongs to the slice that routes
the renderer through the arbiter. Nothing here claims it is in force.

### Completed slice — EVD-0017, the arbiter's publication cost

ADR-0046 names this cost as owed: "The one-arbiter design makes publication serial work on the audio thread. Phase 3
must measure its bounded cost." [EVD-0017](evidence/phase-03/EVD-0017-publication-cost.md) is `Supported`, and its
method — question, falsifier and acceptance rule — was written and committed to before any figure was taken.

At the admitted maximum the pass takes **0.014 % of the callback budget** against a 10 % falsifier, so publication is
an accounting problem rather than a design one. Per-event cost is 1.44–1.90 ns across profiles from 64 to 4 096
frames, with an interquartile range of 2–4 % of the minimum.

**One acceptance criterion is qualified rather than met.** The rule asked for linear per-event cost; the observation
rises 32 % from the smallest batch to the largest. The algorithm is a linear pass, so this is the working set growing
past L2 rather than a superlinear term — but it is recorded as a qualification because a reader extrapolating from
the smallest row would understate the largest by a third.

**It does not reselect `max_events_per_quantum`**, and the host-profile specification's deferred row stays open. That
reselection needs a measured *partition*, and four of the six producer classes have nothing to measure yet. The figure
is a floor for the same reason, and a lower bound again because the harness runs with no callback deadline.

One correction is recorded in the evidence rather than quietly fixed: a first reading called the control arm's figure
"below memset speed" and suspected the ledger clear had been optimised away. That used DRAM bandwidth for an
L1-resident buffer and was wrong.

### Completed slice — compiled admission over anchor phases and loops

ADR-0046 clause 4, and it closes a real gap rather than adding a check. `CompiledEventScheduler::prepare` counted
events per **absolute quantum**, which is the wrong question: two events at frames 63 and 64 are in different quanta
from an anchor at zero and in the *same* quantum from an anchor one frame later. A plan admitted that way faults at
publication after an ordinary seek. The clause says it directly — admission rejects a plan if "any window of `Q`
consecutive integer frame positions contains more events than the share", which "is exactly the worst case over all
`Q` integer anchor phases".

Only windows beginning at an event need checking, because sliding forward without passing an event cannot add one.
That is what collapses `Q` anchor phases into a single pass.

**A loop is a periodic stream, not a window over the plan.** At the wrap the tail of one pass and the head of the next
fall inside one window, and a loop shorter than `Q` puts several whole passes there — clause 4's "loops shorter than
`Q` are not a special hole". The extension repeats `ceil(Q / loop_length) + 2` copies: those wholly inside a window,
plus the one straddling each end. Once this passes, a wrap cannot fail for compiled capacity and the audio thread does
no wrap-time work at all.

The check runs **off** the audio thread, as clause 4 requires: it is finite but its cost scales with the events inside
the loop interval, which no profile capacity bounds — only the window it slides is bounded by `Q`.

Nine checks, three mutation-verified: a closed window instead of a half-open one, two extension copies instead of
`ceil(Q / L) + 2`, and treating the loop's exclusive end as inside. The third needed the test rewritten — the first
version put every event *past* the end, where `<` and `<=` agree, and the mutation passed it. It now places an event
exactly on the end frame, which is one loop length after the start and would collide with the next pass.

Not yet wired into `CompiledEventScheduler::prepare`, which still applies the narrower per-quantum check. Routing it
is the next slice; the module's own doc says so rather than implying the gap is closed.

### Completed slice — the session anchor

The four re-anchoring moments the master plan names: play, seek, loop wrap and offline range start. `SessionScheduler`
owns the current `StreamAnchor` and the tempo map behind it, so a caller goes musical → plan → engine and there is no
musical → engine shortcut to reach for. ADR-0032 clause 27's "anchoring is the only place the two vocabularies meet",
enforced by what exists.

**A loop wrap re-anchors rather than adjusting a position, and that distinction is load-bearing.** Clause 27 says a
position before the anchor "is a scheduler error rather than a clamp", and names the wrap as the case that produces
one. The reason is sharper than staleness: a wrap moves plan time backwards while engine time keeps moving forward,
so the old pairing is **contradictory** — it maps the loop's second pass onto the first pass's frames. A scheduler
that subtracted a loop length would keep answering, and every answer after the first wrap would be wrong by exactly
one loop.

A position before the anchor is refused with both sides named. Clamping would answer with the anchor's own frame,
which is a plausible number and the wrong one: every event before the anchor would pile onto one sample.

**Tempo-map replacement is atomic**, and its interface says what "keep playing" cannot mean. A new map moves every
musical position, so continuing to sound at the same instant *is* a re-anchor; the caller supplies the tick and the
engine time it keeps. The new map is validated before anything is swapped, so a failure leaves the old map and the
old anchor exactly as they were — the master plan's "failure leaves the old map and plan active". A partial
activation would leave events whose engine times were computed under a tempo no longer in force, and nothing would
report it.

Nine checks, three mutation-verified: a wrap that adjusts instead of re-anchoring, a clamp in place of the pre-anchor
refusal, and a replacement that swaps before validating.

**Still owed in this stream:** re-admitting compiled event entitlements against a replacement map. The protocol is
here — validate, then activate atomically — but the entitlement half needs compiled events expressed in musical time,
which they are not: `CompiledEvent` carries `SampleTime`, so recomputing it under a new map belongs with Phase 4's
lowering. Recorded rather than left as an assumed gap.

### Completed slice — the tempo map, steps only

Musical time to [`PlanPosition`], and nothing else. ADR-0032 clause 27 makes anchoring the only place plan time and
engine time meet, so the module has **no** `SampleTime` in its API — the master plan's "the tempo map produces plan
positions; it never produces engine times", enforced by the type rather than by convention.

**Ramps are deferred, and finding out why was the substance of this slice.** A first draft ported V1's ramp
faithfully: tempo linear in tick space, so elapsed time is the integral of `60/bpm` over a linear `bpm`, which is a
logarithm. An independent review caught that **ADR-0032 clause 15 forbids it** — the conversion law "must be
expressible in operations whose results are identical on every supported target", because "a tempo ramp implemented
through a transcendental function would make the frame a note lands on depend on the platform's libm, which the
determinism digest cannot tolerate". A faithful port of V1 and V2's own accepted timing contract are in direct
conflict here.

Clause 15 leaves two ways out — state the exact evaluation, or use a shape the four operations express — and both are
durable choices: the first is a numeric law to specify and test, the second changes delivered musical behaviour (a
60→120 ramp over four beats runs 2.77 s under V1's shape and 3.00 s under a period-linear one). **The maintainer chose
to defer ramps rather than pick one under time pressure.** `TempoChange::ramp` does not exist; a map cannot be built
with one. No project in this repository sets a ramp, so steps are what the corpus actually uses.

**One rounding, and the error is bounded rather than absent.** Clause 15 rounds musical time to a frame exactly once,
half away from zero. `position_of` sums the stored per-segment prefix and the offset inside the segment in seconds and
rounds that sum once — rounding each boundary and adding integers would instead accrue up to half a frame per tempo
change. What does accumulate is `f64` addition over the prefix: about `1e-12` seconds over a ten-minute plan, roughly
`5e-8` frames. An earlier draft of this note claimed nothing accumulated, which was false.

**Conversion past `2^53` is refused, not answered.** Beyond it `f64` stops representing consecutive integers, so two
distinct ticks would map to one position. Both the tick and the position it produces are guarded, and neither
subsumes the other — a tick inside the bound can still produce a position outside it.

Twelve checks. Four mutation-verified against specific wrong answers: counting beats from tick zero rather than the
segment start, truncating instead of rounding, and reintroducing a transcendental. The rounding one needed a fixture
built for it — every other value in the suite lands on an exact frame, so truncation passed all of them until a
6 000 BPM map put a tick at exactly half a frame. `the_conversion_uses_only_the_four_operations` is a standing source
scan in the spirit of the render loop's purity check, so a ramp cannot arrive later by quietly calling a library.

Still owed in this stream: anchoring `PlanPosition` to `SampleTime` at play, seek, loop wrap and offline range start;
recompiling and re-admitting entitlements before a tempo-map replacement activates; and the ramp law itself.

### Completed slice — the compiled producer publishes through the arbiter

The integration the parked ingress slice was waiting for. `CompiledEventScheduler` no longer hands the renderer a
borrowed slice of its own list: it charges every due event to `ProducerClass::Compiled`, seals, and the renderer is
presented the sealed batch. ADR-0046 clause 2's "the only normal path that constructs renderer input" was false for
the one producer the crate already had, and now is not.

Two consequences follow, and both are contract corrections rather than refactoring:

- **The compiled producer's bound is its share, not the cap — at both levels.** Clause 1 partitions
  `max_events_per_quantum`, so the compiled class spends `compiled_event_share`. `CompiledEventScheduler::prepare`
  validates against it, the plan carries it (copied in at admission, since `HOST-INV-001` keeps the profile off the
  audio thread), and **plan admission checks the declaration against it too**. That last one was a defect an
  independent review found: moving the scheduler's bound while leaving admission on the cap left a plan that could be
  admitted and then fault at publication, which is the state clause 3 exists to remove. `PlanDeclarations`'
  statically knowable per-quantum count is compiled work, and its documentation now says so — an aggregate without
  producer attribution could not be checked against any share.
  As a consequence `max_events_per_quantum` is no longer a limit a plan requests: it cannot be exceeded without a
  share being exceeded first, so the refusal case moves to `compiled_event_share`.
- **Clause 7's terminal response is enacted, not merely reported.** A publication fault silences the complete current
  callback and every later one in the epoch, invalidates both carries, publishes `needs_reprepare` and increments an
  **attributable counter** — the clause asks for that last one because a stream that ended for a contract violation
  and one that ended otherwise are indistinguishable without it. It reuses the renderer's own `fault`, because a
  second implementation of "end the stream" is how the two would drift.

Four properties are mutation-verified, each caught by its own check: returning the fault without faulting the
renderer, on **both** the window and the charge branch, since one test could not see the other; charging to the wrong
class; and neutralising the fault counter. Both forged cases are clause 7's own third cause — a caller bypassing the
contract — since no conforming producer can reach either.

**A producer cannot silently change arbiters mid-stream**, which is a weaker property than clause 2's "exactly one
per stream" and is stated as the weaker one deliberately. A bare parameter established nothing: a caller could hand
each callback a fresh, equally sized store, satisfy every capacity bound, and restart the high-water history the
outstanding measurement is going to read. Arbiters now carry a non-reissuing identity — strictly increasing, refusing
permanently on exhaustion rather than wrapping — and a schedule latches the first it publishes through, refusing a
second. **What that does not do is make two schedulers on one stream share an arbiter.** Enforcing clause 2 in full
needs a stream owner that does not exist yet; this latch closes the single-producer case and no more.

A substitution is refused *before* publication and costs the stream nothing: it is a caller error, not a contract
violation, and the two must not share a response. A retry into an already-faulted epoch is likewise not a second
violation — the scheduler refuses to publish there, so `publication_faults` keeps saying the stream ended once.

**What remains in this stream:** the renderer-internal arena, which clause 2 keeps on the far side of the seal; and
`Renderer::render` still accepting a caller-supplied span, which the host-profile specification keeps as Phase 1 and
Phase 2's contract. Live ingress is the parked slice below.

### Parked, with its findings kept — simulated-ingress equivalence

An attempt at the exit gate's simulated-ingress bullet was built, independently reviewed, and **discarded before
commit** on the maintainer's decision. It is recorded here because the review established an ordering fact that the
next attempt must not rediscover.

**The bullet cannot be met before the arbiter integration.** `CompiledEventScheduler` passes its borrowed slice
straight to `Renderer::render`, bypassing the arbiter entirely. An equivalence test therefore compares
ingress-through-the-arbiter against compiled-through-a-bypass — not the two producers at one boundary, which is what
ADR-0046 makes the boundary mean. Such a test stays green even if the compiled-to-arbiter integration is missing or
wrong.

**It also cannot be met before note identity.** An ingress note-on must acquire its release hold atomically with its
queue slot (ADR-0046 clause 3). Without it, filling the queue drops the matching note-off as a new event and leaves
the gate held — and a fixture that proves sample-accurate placement has to use note edges, so the defect is live
exactly where the gate is tested.

Four other findings the next attempt inherits:

- **The forward horizon must be checked at `offer`.** `HOST-INV-013` and ADR-0032 clause 21 put admission at ingress;
  a producer that only forwards events already inside the imminent window never exercises the horizon at all.
- **Late and future destinations are different.** A late accepted entry must still reach ADR-0043's preserving late
  clamp; stopping a drain on both makes it stuck forever. And with non-monotone offers, a future entry at the head
  blocks a due entry behind it, so an accepted entry would wait for *another* entry's destination.
- **An off-thread producer and an on-thread drain need split handles**, not one `&mut self` over a `Vec`. Anything
  else is either single-threaded or a lock the real-time rules forbid.
- **`TimeSource` has no value for a simulated producer**, and the choice stops being provisional once a public
  component ships it with tests asserting it. ADR-0032 clause 18 fixes three: `Hardware` means a driver's timestamp
  bridged through clause 13 and [EVD-0016](evidence/phase-03/EVD-0016-host-time-mapping.md)'s **F11** names labelling
  one without that bridge as a defect; `Compiled` is exempt from the horizon; `Arrival` understates exactness. This
  needs a decision before ingress is public, not at Phase 9.

One methodological finding is worth keeping on its own: an earlier draft of that test moved a **sine's frequency**,
and displacing every ingress event by one frame did not fail it. A frequency is control-rate, so ADR-0001 clause 14
makes it take effect at the next quantum boundary either way. Only a **sample-positioned** payload — a note edge —
makes a one-frame error observable. A placement test built on a control-rate parameter measures nothing.

### Approved: `synth_engine_v2` API breaks during Phase 3

The maintainer approved, on 2026-08-26, the API breaks the producer-share and ingress-store slices make to
`synth_engine_v2::profile`: `EventLimits::new` changed signature twice, and `command_queue_capacity` and
`event_egress_capacity` moved behind `events().queues()`. `AGENTS.md` requires explicit approval for an API break,
and the first of those breaks was committed in `beddf91b` before the approval was sought — recorded here rather than
left implicit.

The approval is bounded by what it was given for: this crate is experimental and is not a dependency of the
workspace's default members, so it has no in-repo consumer outside its own tests. It is not a standing licence for
persisted, manifest, wire or protocol contracts, which `AGENTS.md` treats separately, and it does not reach any other
crate. ADR-0020 settles the final crate boundaries and names.

**The note-payload slice breaks a second set of signatures, and they are outside that approval.** An independent
review raised it, correctly: the approval above names `synth_engine_v2::profile`, and the following are elsewhere.
`EventPayload::Note` changed from `{ slot, edge }` to `{ identity, edge }`; `NoteEdge::On` gained the node the
release lost; `CompiledEvent::new` and `OfflineEvent::new` take a `CompiledPayload` rather than an `EventPayload`;
`CompiledEventScheduler::prepare` takes `&mut PreparedRenderer`; and `stamp_compiled` is new and public.

They are not incidental — they *are* what ADR-0047 clause 1 asks for. A release cannot carry the occurrence alone
while `EventPayload::Note` carries a node, and the pre-stamp payload has to be a different type because an occurrence
does not exist until stamping.

**The maintainer approved this second set on 2026-08-26**, asked for separately rather than presumed covered by the
approval above. The same bound applies and is the reason it was grantable: the crate has no in-repo consumer outside
its own tests, and nothing persisted, manifest, wire or protocol is touched. The approval reaches these five
signatures and no others; a further break asks again.

### Completed slice — a plan declares its note-on producers

The prerequisite the event half was blocked on. ADR-0046 partitions hold entitlements "at plan admission" across
"every admitted non-compiled note-on producer", and ADR-0047 clause 3 partitions identity ranges across a **superset**
of those — but `PlanDeclarations` named no producers, so there was nothing for admission to partition.

`NoteProducerDeclaration` carries two numbers rather than one, because they bound different resources with different
owners: `simultaneous_notes` sizes the **identity range**, which every note-on consumes, and `simultaneous_holds`
sizes the **hold entitlement**, which only a note-on whose release is not already in the same sealed batch consumes.

Four rules, and each is checked where it can be answered:

- **Holds are at most notes.** A hold is taken *by* a note-on, so they are a subset rather than a second budget.
- **A compiled producer declares no hold at all** — ADR-0046 clause 6: "Compiled releases use plan entitlements and
  need no hold." One that asked would consume capacity the non-compiled producers are entitled to. Both of these are
  refused **by name**, before anything is summed, so the caller learns which producer rather than reading a total.
- **The hold partition sums the non-compiled producers** against `release_hold_capacity`. Checking one at a time is
  not admission: two that each fit can together exceed it, which is the rule the record states for authored envelopes
  and which holds here for the same reason.
- **The identity partition sums every producer, compiled included**, against `max_held_notes`. Filtering compiled
  sources out would admit a plan whose compiled notes alone outrun what an identity can name.

`release_hold_capacity` therefore leaves the not-admission-checked list: a plan can now exceed it, so it is a limit
rather than a field the report echoes. Twenty-nine fields qualify, twenty-one do not.

Three properties are mutation-verified: filtering compiled producers out of the identity sum, taking the maximum
instead of the sum for holds, and letting a compiled producer declare one. The row-order test caught the hold row
being emitted out of `ResourceField::ALL`'s position — the third time that check has earned its place.

### Completed slice — the identity table

`SOUND-INV-017`'s first half, and the one point 7 was blocked on. `IdentityTable` mints identities from disjoint
per-producer ranges, resolves them, and retires an index whose generation space runs out.

**The three orphan branches are each covered, separately.** A free index, a live index at a superseded generation, and
a retired index — the third is the one an earlier draft of the specification had no rule for, and the reason the
orphan rule is stated as a definition rather than a list. Resolving without comparing the generation, restarting a
generation instead of retiring, and overlapping the producer ranges each fail their own check.

**The generation ceiling is a construction parameter, and that came from the test.** Walking a `u32` to its ceiling
by minting would take longer than this project will exist, so retirement was unreachable — and a rule no test can
reach is a rule nobody has checked. Making the width a parameter is also what ADR-0047 says it is: a measured
liveness choice, not a safety one, since a generation value is never reused whatever the ceiling.

A foreign-table identity resolves as **foreign, not orphan**: it says nothing about whether the note it named is
live, only that this table cannot answer.

**Two conditions that look alike are kept apart.** A producer with no free index has either over-emitted — every
index live, nothing lost — which is a producer defect, or had its range **eroded** by retirement, which is not: it
declared correctly and did not over-emit. An earlier revision reported both as one error, which would have sent
someone to fix a producer that was behaving.

**A mass release is scoped.** ADR-0046 clause 6 applies the operation "to owned voices within the source event", so a
sustain lift on one source must not end another's notes; only a panic or transport stop reaches everything. An
earlier revision ended everything unconditionally while its own documentation claimed sustain lift.

**A rebuild is refused while an obligation is outstanding**, which is `SOUND-INV-017`'s rule and ADR-0046 clause 3's
reason: rejecting the eventual release would refuse an accepted obligation, and stranding it would leave a note
nothing can release.

**The file is split and `identity/hot.rs` is now in the real-time purity region.** The region is file-granular and
admits no mixed hot/preparation file, and construction allocates — so `resolve`, `release`, `release_all` and
`note_of` live in their own file with a positive control and an import floor, while `mint` stays in `table.rs`,
off the audio thread, where `HOST-INV-009` puts the atomic slot-hold-identity acquisition.

**The occurrence remembers the node**, which is what lets a release carry the identity alone. `SOUND-INV-017` removes
the node address from a release rather than carrying it and requiring agreement, so something has to remember it —
and the occurrence is the only thing that can, because the node was named when the occurrence was created. `mint`
therefore takes the node, and `note_of` returns it for a live identity and nothing for any other.

One cost is worth stating before that slice rather than after: `release_all(Everything)` scans every slot, and that
is the **sum of the admitted producer ranges** — not `max_held_notes`, which bounds simultaneous obligations rather
than the extent handed out. A panic's cost therefore scales with the admitted extent, up to the whole index space,
independently of how many notes are sounding. An earlier note here said `max_held_notes`, which is wrong.

**What this does not do, and the prerequisite that stops it.** `EventPayload::Note` still carries `{ slot, edge }`,
so nothing asserts that a note-on names an occurrence or that a release names one alone, and the conformance row says
exactly that.

Changing the payload needs the renderer to hold an identity table, and building one needs the **producer ranges** —
which ADR-0046 partitions "at plan admission" across "every admitted non-compiled note-on producer". **A plan
declares no such producers.** `PlanDeclarations` has no note-on producer set, so there is nothing for admission to
partition, and a renderer cannot be given a table without inventing one.

Two ways out, and neither is this slice's to pick unasked. Phase 3 today has exactly one note-on producer, the
compiled one, so an interim table with a single range would work and would have to be documented as interim. Or
`PlanDeclarations` gains the producer set now, which is the shape ADR-0046 assumes and which Phase 5's authored
sources will need anyway. The second is more work and less likely to be redone.

### Accepted — ADR-0047 note identity, with its specification transaction

[ADR-0047](decisions/ADR-0047-note-identity-in-the-event-contract.md) is `Accepted`. It exists because ADR-0046
clause 3 already promises that an orphan note edge "is counted rather than allowed to release another note", and the
`{ slot, edge }` vocabulary cannot tell an orphan from a legitimate release — so identity is a Phase 3 requirement,
not preparation for Phase 6.

Acceptance updated three current specifications in the same transaction:

- **`SOUND-INV-017` is new.** A note **on** names an occurrence as well as a node; a release names the occurrence
  **alone**, and the occurrence is the sole authority for which note an event resolves to. Carrying both on a release
  would admit an event whose identity and node disagree, and no reading of that is safe, so the case is removed
  rather than adjudicated. `SOUND-INV-016` was narrowed in the same transaction — it said "a note event names a node"
  where it meant the on edge, which would otherwise have left the specification requiring and forbidding the same
  thing.
- **`HOST-INV-009` was amended, not extended.** Its closed list licensed two live-input drop causes and said no other
  shortage may be discharged as a drop; an exhausted identity range is a third, and the exhausted-resource name a
  report carries is now slot, hold **or** identity. Two further causes are counted there and are explicitly *not*
  drops — an orphan and a never-minted edge are refusals, and reporting one as a drop would make a producer look
  starved when it is releasing a note that does not exist. An orphan is defined as *an identity naming no live note*,
  with three reachable cases: a free index, a superseded generation, and a retired one. An earlier draft listed only
  the first two, which left an implementer no rule for the state ADR-0047's own retirement clause creates.
- **`HOST-INV-021`'s hold contract names the identity** as what a hold is acquired against and redeemed by, and the
  construction relations gained ADR-0047 clause 5's index bound: the identity index space is at least
  `max_held_notes`, which is otherwise constrained only to be nonzero.

**`SOUND-INV-017`'s table half was checked first; the event half followed in the slice below.** This slice supplied
the identity type and its rules; the payload change that consumes them is separate and is recorded on its own.

**REV-P02's `NoteEdge` deviation row is still open.** ADR-0047 adds identity and discharges neither limb; the row
keeps the owner and the "owed before ingress" deadline REV-P02 gave it. Its pitch limb is blocked on ADR-0025, which
is `Proposed` for Phase 6, so Phase 3 must either accept that record early or change REV-P02's disposition
explicitly. Its velocity limb carries no such coupling. That choice is open and belongs to the maintainer.

### Completed slice — a note event carries its occurrence

`SOUND-INV-017`'s event half, and what closes the last of ADR-0047's implementation debt short of ingress.
`EventPayload::Note` now carries `{ identity, edge }` where `NoteEdge::On` carries the node and `NoteEdge::Off`
carries nothing. The release has no node field to disagree with its identity, which is the case ADR-0047 removed
rather than adjudicated.

**Pairing moved to a boundary it can still be answered at.** After stamping, a release carries only an occurrence, so
"which note-on does this release end" has no answer left in the list. A new `CompiledPayload` — `SetParameter`,
`NoteOn { slot }`, `NoteOff { slot }` — is what a compiled list is written in *before* stamping, and one shared
`schedule::stamp_compiled` mints the occurrences and pairs the edges for both the compiled scheduler and the offline
renderer. Two implementations of that question is how they come to disagree, so there is one. `stamp_compiled` is
public because it is the only sanctioned way to obtain an identity: the ranges are the plan's, partitioned at
admission, and a producer that minted its own would land outside the partition every disjointness check relies on.
It takes no epoch — it derives the renderer's own, because a list stamped against another stream's epoch would
succeed, reserve this producer's range, and then be discarded event by event as stale.

**Minting and liveness are two jobs, and the slice's first version conflated them.** `IdentityTable` models a
producer's *occupancy*: an index is taken at a note-on and freed at the release that pairs with it. Stamping runs
ahead of the render — for a whole piece at once — so the table's state at stamping time is the schedule's polyphony,
which is exactly what `simultaneous_notes` bounds. It is **not** what is sounding when an event is applied, and a
reissued index is where the two disagree. The renderer therefore keeps `LiveNotes`, a registry the *events* write: a
note-on admits its occurrence together with the node its edge names, and a release resolves through it and clears it.
So both notes that used one index resolve correctly, in the order the renderer applies them.

That resolution happens **once per call**, in a pass that runs after the events are sorted and before the per-quantum
passes, and the resolved node and control are cached on the scratch event. The two quantum passes must agree — the
first counts what the second writes — and a registry mutated between them would break that. Nothing in the render
loop touches the registry any more.

**Four defects the slice found in itself, and six an independent review found after it.** The plan carried the
producers' *ranges* but not which of them was the compiled one, so stamping took producer 0 — correct in every fixture
and wrong in any plan declaring a runtime source first; `CompiledPlan` now carries `compiled_note_producer`, and
admission refuses a **second** compiled producer, because `PlanDeclarations::events_per_quantum` is one figure against
one share and a second producer would leave both the envelope and the minting range a guess. The renderer's foreign
filter compares the occurrence's **table** rather than a slot, so a note-on carrying another plan's node address would
have passed it; `stamp_compiled` refuses that, at the last point the node is present and the one `render_offline`
reaches without the scheduler's list-wide check. A foreign **parameter** slot is deliberately still filtered and
counted at render rather than refused: that is the documented post-swap behaviour, and `lowering` asserts it.

The review found the occupancy defect above, and four more that the split does not by itself fix, each now closed:
stamping is **all or nothing**, so a refused list leaves the minter as it found it — minting as it walked would have
starved the next, valid attempt; an orphan release is **counted**, not silently skipped, because `SOUND-INV-017`
requires refused *and* counted and a producer replaying spent releases would otherwise look like one sending nothing;
the epoch is derived rather than accepted; and the scratch budget now includes both identity halves, which
preparation allocates per admitted index.

A second review round found two more. "All or nothing" was **not** achieved by validating first: pairing, provenance
and producer presence are decidable before the first mint, but minting can fail on its own — a list can pair
correctly and still hold more notes at once than the range admits — and a check for that beforehand would have to
reimplement allocation. The minting pass therefore works on `IdentityTable::working_copy` and assigns it back only on
success. Restoring by releasing what was minted would not restore: a release advances the generation, and the paired
releases an aborted list already performed are not recoverable that way.

And the orphan counter was anonymous where ADR-0047 clause 4 asks for the event to be counted "against its offering
producer with the identity named". `DiagnosticsReport::last_orphan_note` names one. **Naming the identity names the
producer** — the ranges are disjoint and a producer's position in the declaration is its `ProducerId`, so the index
falls in exactly one range. What is owed is *per-producer counts*, and it is owed to ingress: until a runtime note-on
producer exists, every reachable plan has exactly one note producer, so the aggregate **is** that producer's count.
A second producer is what makes it ambiguous, and the conformance row says so rather than implying coverage.

Thirteen properties are mutation-verified, listed in the render contract's conformance row. The falsifiable fixture is
worth naming: two gates in **series**, told apart by release *shape* rather than by level, because the IR has no mixer
and two sustain levels through one product render identically — a release resolved to the wrong note would have
passed every level-based assertion.

EVD-0013's thirty-four V2 renders are bit-identical to `2a00685e`, checked against a separate worktree, so the payload
change is behaviour-preserving on the equivalence arm.

`orphan_note_events` joins `DiagnosticsReport`. It is deliberately **not** a drop: `HOST-INV-009` licenses a drop for a
shortage, and an orphan is a release for a note that does not exist — reporting one as a drop would make a producer
look starved when it is not. That is the amendment ADR-0047's specification transaction already made to the invariant;
this is the counter it named.

## Paused parallel stream: Phase 0B

Outcome: complete the V1 migration inventories and the durable Project and Application Core contracts required before
Phase 10.

This phase remains active in the roadmap, and its execution stream is paused.
It was paused for the Phase 2 slice, which has now closed, so **nothing is
holding it any longer** — resuming it is a choice rather than a wait. On
resumption, select exactly one task, copy its observable completion check here,
and only then mark it `Active`.

| Task           | State       | Resume boundary                                                            |
|----------------|-------------|----------------------------------------------------------------------------|
| P00B-T001      | Paused      | Assign evidenced dispositions in the state-ownership inventory             |
| P00B-T002      | Paused      | Assign reachability and migration dispositions in the capability inventory |
| P00B-T003      | Paused      | Resolve the two format questions that block ADR-0014 review                |
| P00B-T004–T007, P00B-T009 | Not started | Follow the decomposition in the frozen execution record       |
| P00B-T008      | Not started | Re-scope the frozen all-ADR task under `PROCESS.md`'s decision-timing rule  |

This stream does not block Phase 2. Its detailed audit chronology remains in
the [historical Phase 0B execution record](phases/phase-00b-inventories-and-project-contracts.md); new operational state
is recorded only here.

Phase lifecycle and completed gates are recorded once in
[`ROADMAP.md`](ROADMAP.md#phase-order).

## Later owned work

- Phase 3 owns renderer ingress, the publication arbiter and producer shares,
  event scheduling, and capacity measurements. Its exit work
  also owns ADR-0043's named offline late-clamp test:
  prove the stamp-window selector cannot present a late event, or window by
  clamped render position. That test is not an entry prerequisite.
- Phase 4 owns current-project lowering and the long-running job contract.
- Phase 5 owns the `LegacyPolyModuleAdapter`'s conversion cost — the largest quantity ADR-0041 moves and the only one
  nobody has measured — and the declarative node API that `SOUND-INV-012`'s uncovered second sentence belongs to.
- Phase 9 owns completion and acceptance of ADR-0022 against retained evidence for every claimed release platform and
  initial adapter. Phase 9 may build candidates while the record is `Deferred`, but cannot exit or qualify live timing.
- Phase 0B gates Phase 10.
- ADR-0039 and `LIMIT-0017` remain Phase 10E work.
