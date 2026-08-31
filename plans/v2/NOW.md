# Core V2: Current Work

Last updated: 2026-08-29

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

The linear half is now what preparation is built on, through the admitted-stream slice below, and the loop half's
diagnostic is complete as of the slice after it. The loop half still has no caller: a loop interval is transport
state, and `SessionScheduler` re-anchors at a wrap without carrying one.

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
with one. That note also said no project in this repository sets a ramp; **it was false**, and ADR-0049's evidence
is where it was found. `CORPUS-0001`'s `tempo-map-arrangement` ramps 90 to 180 BPM over two beats. The narrower true
statement is that no project under `assets/examples/` sets one.

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
and recompiling and re-admitting entitlements before a tempo-map replacement activates. The ramp law itself is closed
by the slice below.

### Accepted — ADR-0049 the tempo-ramp law, with the ramp built

[ADR-0049](decisions/ADR-0049-tempo-ramp-law.md) is `Accepted` and `TempoChange::ramp` exists. A ramp interpolates
the **period** — seconds per beat — so elapsed time is a quadratic the four operations express exactly. The
maintainer chose it on 2026-08-31 over stating a bit-exact evaluation of V1's logarithm, and chose it **again** after
the corpus finding below, in the knowledge that the fixture's timing moves.

**The deferral rested partly on a false sentence, and finding that out is what changed the question.** The tempo
slice recorded that no project in this repository sets a ramp. `CORPUS-0001`'s `tempo-map-arrangement` sets one: 90
to 180 BPM over two beats. Under the accepted law that segment lasts 1.000 s against V1's 0.924 s, so everything
after tick 1920 moves by 75.8 ms — 3 639 frames at 48 kHz. That is an **intentional V1-to-V2 semantic change**, which
`REV-P00A`'s rule requires to map to a comparison category rather than to generic error, and Phase 4's A/B owns
creating it. ADR-0042's envelope shape is the precedent.

**One near-branch removed rather than reproduced.** V1 switches to a constant tempo at the endpoint average when
`|b1 - b0| < 1e-5`, to avoid a `0/0` cancellation its logarithm has. The quadratic has no cancellation: an
equal-endpoint ramp's second term is exactly zero and the answer is the step law's own, bit for bit. That identity
comes from the two laws **sharing** one linear term rather than from any particular evaluation order.

**A draft of this record claimed the order itself was load-bearing, and a mutation refuted it.** Rewriting the linear
term as `period * beats` passed all eighteen checks; four million random musical `(tempo, tick)` pairs then produced
no case where the two orders reach different frames. The claim was withdrawn from the ADR, the specification, the
module documentation and the test's own comment. It is recorded because the mutation is what caught it — rereading
the sentence would not have.

**The independent review found two P2 defects; a focused reread of the repairs found four more, and one of them
turned a repair into a withdrawal.** The chronology matters because the outcome is a *smaller* contract than the
first draft claimed, arrived at by being unable to defend the larger one.

- **An accelerating ramp could move a position backwards.** A 20-to-6000 BPM ramp ending at tick
  `100_000_000_000_000` converted two adjacent ticks to *decreasing* frames, both inside the `2^53` guard and both
  tempi ones this suite already uses. The first repair rewrote the accelerating case as
  `p1 * beta + (p0 - p1) * beta * (2B - beta) / (2B)`, whose terms are both non-negative. The reread constructed a
  counterexample against **that** too, at a different tick in the same ramp.
- **So the guarantee is withdrawn, and the reason is a measured trade rather than an inability to find a form.** A
  third draft claimed no algebraic form could be a composition of monotone operations; **the reviewer refuted that
  by writing one** — the rising case taken relative to the segment's end, whose bracket is non-increasing. Measuring
  it is what settled the question: near the segment start it subtracts two large quantities the accepted form never
  forms, and over random ramps that cancellation placed a segment's own start as much as **128 seconds before its
  own prefix**. Trading a one-frame artefact at forty-two years of audio for a two-minute artefact at the start of
  every steep ramp is the wrong direction. **How often that alternative errs is not claimed either**: the
  128-second case is a ramp between two very slow tempi over the whole tick range, while an ordinary 20-to-6000 BPM
  ramp errs by about `1.8e-15` seconds and the ratio-five-thousand construction by nothing at all. What decides the
  trade is bounded-and-sampled against unbounded, not a frequency. ADR-0049 clause 6 now claims a **sampled**
  property — adjacent ticks in four windows of steep ramps, since a stride of 97 steps straight over the one-frame
  inversion it was supposed to catch, which the reviewer also found — and records both revisit routes.
- **Every threshold I named was wrong, so the records now name none.** Three drafts said inversions need a position
  above `2^53`, then `2^49`, then `2^45.85` frames; the reviewer refuted each with a better search than the one that
  produced it. A bound that moves every time someone looks harder is not a bound. What the records state instead is
  the shape of the domain — a position decades of audio out **and** a tempo ratio in the thousands, both needed —
  which no counterexample can falsify because every counterexample so far confirms it.
- **A ramp could build a `Bpm` its own constructor refuses, four ways.** A tempo below `60 / f64::MAX` has a period
  that overflows, so the ramp evaluated `infinity * 0` and produced `NaN`; closing that at the newtype exposed an
  intermediate overflow in the interpolation; forming the fraction first exposed a third — for a 6000-to-`1e100` BPM
  ramp the fraction reaches exactly `1.0` one tick before the end, `(p1 - p0)` rounds to `-p0`, and the sum is
  exactly zero; and the weighted form alone can round one unit in the last place below the endpoint period, whose
  reciprocal overflows for a tempo near `f64::MAX`. A positive finite denominator does not imply a finite
  reciprocal. The interpolation now weights the two periods **and clamps to the interval they define**, which is
  where a convex combination lies and where only rounding can take it out of. The narrowing of `Bpm::new` is a
  behaviour change, named rather than left to be found; the refused range is one beat per far longer than the age of
  the universe, and the diagnostic now names the period as the reason.
- **The source scan could hide a real call, two ways.** Stripping `//` comments — added because the word "rising"
  contains `sin` and failed a correct implementation — truncates a line at a `//` inside a string literal, so
  `("https://x", beats.sin()).1` would lose its call; a line holding a quote is now scanned whole. And the scan
  **did not follow calls at all**, so `segment_seconds` could have called a harmlessly named `curve(x)` whose body
  held `x.ln()`. It is now closed under calls **and under names**: every call the scanned bodies make
  must be to one of the five functions the law reaches or to a named arithmetic or accessor method, and no
  allowlisted name may itself be a function this module defines — otherwise `fn min(x) { x.ln() }` called as
  `min(beats)` passes, which the reviewer pointed out about the first version of the closure. Adding those
  assertions immediately pulled `segment_for` and a newtype accessor into the scanned set, which is the check doing
  its job rather than a nuisance. Both holes are mutation-verified.

**Two checks were written vacuous and are recorded that way**, because the mutation is what found them and reading
them would not have: a four-beat fixture cannot make the elapsed fraction reach exactly `1.0` inside a ramp, and the
module's own source holds no line where a `//` hides inside a string, so neither check could fail for the reason it
named. Both now reach their case, and both fail under their own mutation and no other.

Ten ramp checks in all, each mutation-verified against its own falsifier: the equal-endpoint identity against a ramp
that recomputes the shared term; the corpus fixture's exact 48 000 frames, and explicitly not V1's 44 361, against
the quadratic's sign and against treating every change as a ramp; monotonicity in both directions over an hour,
tick by tick, against that same sign; the period refusal and the steep ramp's reported tempo against dropping the
check and against any of the three rejected interpolation forms; chained ramps each reaching the **next** declared
tempo against pointing one at the last; a trailing ramp behaving as a step against a degenerate destination; the
reported tempo being the reciprocal of the interpolated period — 120 BPM halfway through a 90-to-180 ramp, not the
135 a straight line would give — against reporting the declared tempo; and the scan's call closure against a
transcendental hidden in a helper it does not follow.

**Six review rounds, and the slice is smaller at the end than at the start.** That is the outcome, not an
apology: what shipped is a law whose musical behaviour was never in question, a set of claims narrowed until each
one is checkable, and four numerical defects that only existed because the first draft claimed more than it could
support.

Acceptance creates `SOUND-INV-019`. The **inverse** conversion is deliberately not defined: nothing on the render
path needs it, and inverting this law needs a square root, which clause 15's listed set does not carry. Phase 11 asks
for that amendment if a musical playhead readout wants one.

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

**The note-payload slice breaks a second set of signatures, outside that approval.** Two independent review rounds
raised it: the approval above names `synth_engine_v2::profile`, and none of the following is there.

Type changes:

- `EventPayload::Note` changed from `{ slot, edge }` to `{ identity, edge }`, and `NoteEdge::On` gained the node the
  release lost;
- `CompiledEvent::new` and `OfflineEvent::new` take a `CompiledPayload` rather than an `EventPayload`, and
  `CompiledEvent::payload` and `OfflineEvent::payload` return one;
- `CompiledEventScheduler::prepare` takes `&mut PreparedRenderer` rather than `&PreparedRenderer`.

Added variants on public enums. **None of these types is `#[non_exhaustive]`**, so a variant is a break for an
exhaustive match, which is why they are listed rather than treated as additive:

- `SchedulePrepareError::UnmatchedRelease`, `::NoCompiledNoteProducer`, `::Identity`;
- `OfflineError::Stamp`;
- `CompileError::SecondCompiledProducer` and `::IdentityPartition`.

Making these enums `#[non_exhaustive]` — one break now against every later one — was offered and not taken. If a
future slice wants it, it is a break of its own and asks then.

**The transport-activation slice breaks a third set, approved by the maintainer on 2026-08-27.** An independent
review found the inventory below incomplete, which is why it is stated in full rather than by example. None of it is
in `synth_engine_v2::profile`, so the 2026-08-26 approval did not reach it and a separate one was sought. The grounds
are the same ones that approval rested on: the crate is experimental, is not a dependency of the workspace's default
members, and **nothing that ships depends on it** — `crate_boundary.rs` permits exactly one in-repo consumer,
`pertylizer` as a dev-dependency, and only for the three measurement examples. An earlier wording said "no
in-repo consumer outside its own tests", which that test file contradicts; the verified property is the
narrower one. Making the two enums `#[non_exhaustive]` was offered
again and again declined, so a later variant asks again.

Signature changes:

- `render::timed_control_scratch_bytes` takes the identity partition as a third argument.

A semantic change to existing public accessors, approved on 2026-08-28: `Publication`'s and `SealedBatch`'s
`spent`, `external_total` and the arbiter's high-water marks count **charges** rather than batch entries, because
ADR-0046 clause 6 charges a bounded operation to a share without expanding it into events. A window can therefore
report occupancy against an empty batch. The share's load is what those accessors are for, and a ledger counting only
batch entries would leave part of it invisible; an independent review raised the divergence, and the alternative —
counting operations beside the ledger — would have split the one quantity clause 7 makes an overrun of a contract
violation.

Added variants on public enums, none of which is `#[non_exhaustive]`, so each is a break for an exhaustive match:

- `schedule::SchedulePrepareError::CandidateOutstanding` and `::SchedulerExists`;
- `schedule::ScheduledRenderError::EventTimeUnrepresentable`.

Everything else the slice adds is new surface rather than a break: the whole `transport` module,
`stream::{StreamControl's activation methods, ActivationRequest, ActivationBuildError, ActivationCollectError}`,
`schedule::stamp_into`, `AudioBlockMut::{split_at_frame, reborrow, silence}`,
`CompiledEventScheduler::{offer, collect, in_force, loop_interval}`,
`PreparedRenderer::{control_scratch_bytes, prepared_record_count}`, `Publication::charge_operation`, and three
diagnostics counters. The types introduced *within* this slice changed shape
several times as the reviews ran — `LoopInterval::length`, `StreamControl::{adopted, withdraw}` — and those are not
breaks, because nothing committed has ever seen the earlier shapes.

`CompiledPayload` is a **new** enum rather than a widened one, so its three variants break nothing.

New public surface, which breaks nothing but is part of what was approved: `schedule::stamp_compiled`,
`schedule::CompiledPayload`, `identity::LiveNotes`, `render::identity_bytes`,
`CompiledPlan::{note_producer_ranges, compiled_note_producer}`,
`DiagnosticsReport::{orphan_note_events, last_orphan_note}`, `IdentityTable::from_admitted_ranges`.

They are not incidental — they *are* what ADR-0047 clause 1 asks for. A release cannot carry the occurrence alone
while `EventPayload::Note` carries a node, and the pre-stamp payload has to be a different type because an occurrence
does not exist until stamping.

**The maintainer approved this second set on 2026-08-26**, asked for separately rather than presumed covered by the
approval above. The same bound applies and is the reason it was grantable: the crate has no in-repo consumer outside
its own tests, and nothing persisted, manifest, wire or protocol is touched.

**The enumeration put to the maintainer named five signatures and was incomplete** — a third review found the two
`payload` return types and five error variants, and a fourth found the sixth, `CompileError::IdentityPartition`. The
list above is now derived mechanically, in both halves: variant sets per public enum, and parameter-and-return
signatures per public function, taken from the commit and its parent and differenced — rather than read off the diff
by eye. The signature half came back as exactly the five already listed; `IdentityTable`'s own four methods are
byte-identical across the commit, and an earlier version of that scan reported two of them as changed because it
keyed on file and function name, which `LiveNotes` now shares with them in one file.

Three successive hand-written versions of this list were wrong, and the count in this paragraph disagreed with the
list beneath it — which is the tell that the method was the problem, not the care.

**The first approval covered the five signatures the maintainer was shown; the remaining eight were put to them
separately and approved on 2026-08-26.** Those eight are the two `payload` return types and the six enum variants.
An approval cannot be widened after the fact by the party that asked for it, and a fifth review round caught the
first version of this paragraph doing exactly that — the second request is what closed it rather than the
rewording.

**The admitted-compiled-stream slice breaks a third set, approved on 2026-08-26.** Asked for before the commit rather
than after it, and derived by the same mechanical method: variant sets per public enum and parameter-and-return
signatures per public function, taken from the commit and its parent and differenced.

Signature changes:

- `CompiledEventScheduler::prepare` takes `(&mut PreparedRenderer, StreamAnchor, &AdmittedCompiledStream)` rather
  than `(&mut PreparedRenderer, &[CompiledEvent])`.

Variant changes on `SchedulePrepareError`, which is not `#[non_exhaustive]`, so both halves are breaks:

- **removed** `QuantumTooDense` and `EventsOutOfOrder`, whose proofs moved to admission;
- **added** `AnchorMismatch`, `BeforeAnchor`, `ForeignStream` and `TimeUnrepresentable`.

Two behavioural breaks carry no signature and were approved with the rest:

- a stream is refused when **any** `Q`-frame window is over the compiled share, not only an over-full absolute
  quantum, so streams that used to prepare can now be refused — which is ADR-0046 clause 4's point rather than a
  side effect;
- `AdmissionError::WindowOverShare` names the **first** over-full window rather than the densest. The same streams
  are refused; `window_start` and `requested` differ.

New public surface, which breaks nothing: `schedule::{PlanEvent, AdmittedCompiledStream, CompiledStreamError}` and
`time::{Located, StreamAnchor::locate}`. `PreparedRenderer::anchor` is `pub(crate)` and is deliberately not public
surface — its only sanctioned use is the refusal above.

The bound is the one the earlier approvals were granted under and is why this one was grantable too: the crate is
experimental, is not a dependency of the workspace's default members, and has no in-repo consumer outside its own
tests. Nothing persisted, manifest, wire or protocol is touched, and no ADR is created — ADR-0046 clause 4 already
fixes *that* admission happens over every anchor phase; this slice chooses only the shape, and a later producer
wanting a different one is where that becomes a decision of its own.

**The loop-diagnostic slice breaks one more, approved on 2026-08-26.** `admit::AdmissionError` gains
`LoopWindowOverShare`; the enum is not `#[non_exhaustive]`, so the addition breaks an exhaustive match, and
`admit_loop` now returns it for inputs that previously produced `WindowOverShare`. Additive alongside it:
`time::AnchorPhase` with `of` and `as_u16`. The same bound applies and `admit_loop` had no caller when this slice
landed, so nothing in the repository was affected. The transport-activation slice below is its first caller.

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
falls in exactly one range. What is owed is *per-producer counts*, and it is owed to ingress — but the reason the
aggregate is unambiguous meanwhile is about **emission**, not about how many producers a plan declares.
`stamp_compiled` is the only path that mints into a renderer's table, and it mints only from the plan's compiled
producer, so every occurrence a renderer can see is that producer's. A producer that emits without going through
compiled stamping is what makes the aggregate ambiguous.

A third review round caught the first version of that justification, which argued from the producer *count* and was
contradicted by `note_identity`'s own two-producer fixture in this very commit. It also caught the claim standing in
three places — this section, the conformance row, and the accessor's own documentation — and only one of them
repaired. The withdrawn phrasing was then grepped to zero before the next read, which is what found the third.

Fifteen properties are mutation-verified, listed in the render contract's conformance row. The falsifiable fixture is
worth naming: two gates in **series**, told apart by release *shape* rather than by level, because the IR has no mixer
and two sustain levels through one product render identically — a release resolved to the wrong note would have
passed every level-based assertion.

EVD-0013's thirty-four V2 renders are bit-identical to `2a00685e`, checked against a separate worktree, so the payload
change is behaviour-preserving on the equivalence arm.

`orphan_note_events` joins `DiagnosticsReport`. It is deliberately **not** a drop: `HOST-INV-009` licenses a drop for a
shortage, and an orphan is a release for a note that does not exist — reporting one as a drop would make a producer
look starved when it is not. That is the amendment ADR-0047's specification transaction already made to the invariant;
this is the counter it named.

### Completed slice — compiled admission becomes a value, not a per-call check

ADR-0046 clause 4's linear half, routed — but **not** by calling `admit_linear` from
`CompiledEventScheduler::prepare`, which is what the previous slice's note proposed. An independent consultation
attacked that plan and the objection held: `prepare` receives a caller-supplied list of engine-time events, and
nothing proves the list it is handed at one anchor is the same stream it would be handed at another. **Shift
invariance of one set does not establish anchor independence across different sets.** A check inside `prepare` would
therefore judge each anchor's list on its own and prove nothing about the stream.

Admission is now a **value**. `AdmittedCompiledStream::admit` takes `PlanEvent`s — plan positions, no anchor — and
proves three things: ascending plan order, every slot belonging to the plan, and no `Q`-frame window over
`compiled_event_share`. `prepare` accepts only that type. The set it places is therefore the set that was admitted,
and the artifact admission judges exists before an anchor is chosen, which is what clause 4 means by "the worst case
over all `Q` integer anchor phases".

**Preparation re-checks no capacity.** Two proofs of one property is how they come to disagree, and the one that is
wrong is whichever the caller did not read. What `prepare` does instead is place: the anchor's forward mapping, which
keeps ADR-0032 clause 27's "anchoring is the only place the two vocabularies meet" true of this path. Adding a
constant preserves order, so there is no second ordering check and no second ordering policy either.

**A position before the anchor is refused, not skipped.** The stream begins at the anchor, so such a position is one
this stream does not render — but dropping it silently is what ADR-0001 clause 16 forbids, and the mutation shows
what the silence would look like: skipping the note-on turns its release into `UnmatchedRelease`, which sends someone
to look for a malformed plan. The caller that meant to start there admits the **suffix**, which always fits, because
every window of a suffix is a window of the whole.

`StreamAnchor::locate` exists because `time_of`'s single `None` conflates two answers that need different responses.
A pre-anchor position says the stream does not reach it; an unrepresentable one says the clock ran out and says
nothing about seeking. Reporting the second as the first would send someone looking for a seek that never happened.

**The anchor took two review rounds, and the second reversed part of the first.** A first draft had `prepare` reach
for the renderer's own anchor, which is the one value guaranteed to be stale: clause 27 names seek and loop wrap as
re-anchoring moments, neither re-prepares the renderer — a wrap that dropped the carries would be audible — so
`PreparedRenderer`'s anchor is fixed at preparation while `SessionScheduler`'s moves. Placing against the renderer's
would put a post-seek stream at the pre-seek pairing: shifted, or behind the clock and late on arrival. So the anchor
became an argument, and the accessor went with it.

The next round showed why the accessor could not simply go. **The renderer's anchor is not inert**: its hot path
derives every quantum's plan position from it, so a stream placed at a different pairing runs the events on one
timeline and the position-aware kernels on another — an `Impulse` keeps sounding where the old anchor says while the
notes move. This note's previous revision claimed that anchor was "read by nothing", which was false; it came from
grepping `render.rs` and not `render/hot.rs`, and it is exactly the kind of claim the reviewer is there to catch.

So the anchor stays an argument and preparation **refuses** a placement the renderer is not anchored at. Passing it
explicitly is what makes the refusal expressible at all: the caller states the pairing it means, and preparation says
no when the renderer is elsewhere, instead of one side silently winning. The accessor is back as `pub(crate)`, used
only for that comparison and documented as not a placement source.

**Which names the third owed sub-question: re-anchoring a prepared renderer.** Moving the renderer's mapping and the
scheduled events together is what a seek and a loop wrap actually are, and until it exists neither can be expressed
against a live renderer — a refusal, not a wrong answer.

**Which window the refusal names took two corrections, and the second is the interesting one.**
`HOST-INV-021` asks for "the exact first over-full half-open `Q`-frame window". `AdmissionError::WindowOverShare`
named the **densest** one, which in a plan with an early overrun and a later, denser cluster points past the overrun
that has to be fixed first. Naming the first *event-aligned* over-full window instead — the obvious repair, and the
one this slice made — is still not it, and the independent review caught that: with a share of one and events at 63
and 64, every window from `[1, 65)` to `[63, 127)` holds both, so the first begins at frame **1**. Naming 63 names a
real over-full window and not the earliest, which is to say not the anchor phase at which the stream first fails.

Deciding **whether** a stream fits and deciding **which window to name** are therefore different questions, and only
the first collapses to the event-aligned scan. The named start is now walked back: the window must reach the
`share + 1`-th event, so the earliest start that does is that event's position minus `Q - 1`, and nothing earlier can
be over-full because any over-full window holds `share + 1` consecutive events beginning no earlier. A brute-force
check over every start frame, on four fixtures, is what holds the two in agreement — the derivation is short enough
to believe and was wrong once already.

The review's second finding is the same shape: `StreamAnchor::locate` subtracted before it ordered, so a position
more than `i64::MAX` frames *behind* the anchor was reported as an unrepresentable time rather than a pre-anchor one.
Which side of the anchor it sits on was never in doubt — comparing answers it directly — and ADR-0032 clause 27 asks
for a scheduler error there, not a shrug. Ordering now happens first.

Seven properties are mutation-verified, each caught by a check named for it: counting per absolute quantum instead of
sliding a window, a closed window instead of a half-open one, naming the event-aligned window instead of the earliest,
subtracting before ordering, skipping a pre-anchor position instead of refusing it, admitting against the cap instead
of the compiled share, and dropping the anchor-agreement refusal. The falsifiable fixture
is the one the first of those needs — half a share at frame
`Q - 1` and the rest at frame `Q`, which is under the share in **both** absolute quanta and over it in the window
that straddles them. Its control pins the half-open boundary from the other side: two full shares exactly `Q` apart
share no window and must be admitted.

**Three sub-questions are named rather than closed.** The loop half has no caller, so no part of this claims clause
4's loop obligation — and its diagnostic does not yet name the loop interval and phase that clause 4 requires, which
the consultation found and this slice did not touch. `render_offline` reaches `stamp_compiled` directly, so the
offline path takes no admitted stream; whether offline compiled events consume the same proof is a decision, not an
oversight, and it is unmade. And re-anchoring a prepared renderer, above, is owed before a seek or a loop wrap can be
expressed against a live stream at all.

### Completed slice — a loop's refusal names its interval and a phase

ADR-0046 clause 4's other sentence: "the diagnostic names the loop interval, phase, requested count and available
count". `AdmissionError::WindowOverShare` named none of the first two, and `admit_loop` reported through it — so a
loop refusal carried a `window_start` from the periodic extension and nothing identifying the loop. The gap was found
by the consultation that redirected the admitted-stream slice, and it is closed here rather than left in the record.

**A loop's overrun has no single window to name, and that is why the variant is separate.** A linear stream fails at
one place, so naming its first over-full window points at it. A loop repeats, so an over-full window recurs at every
copy — and because a copy sits `length` frames away, the phase moves with each copy unless `Q` divides the loop
length. The frame is never unique and the phase **need not be** — a loop whose length is a whole number of quanta
keeps one phase, which is exactly what the single-witness fixture below relies on. An earlier revision of this note
and of the code's own documentation said "neither is unique", which that fixture contradicts; a review caught the
contradiction. `LoopWindowOverShare` therefore reports a **witness**: one phase at which one quantum of the periodic
extension holds more than the share, taken from the earliest over-full window the scan reaches.

A start frame is deliberately not reported beside it. It is a position on the plan's axis that can fall **outside**
`[start, end)` — two events on the loop's own first frame put it `Q - 1` frames before the loop — so it names no
place in the looped material.

`Q` is not a field either, where the linear variant carries one. Clause 4's list for a loop is the interval, phase,
requested and available counts, and `Q` is on none of them — it is a crate constant the message names from
`FrameCount::QUANTUM`. A review asked for the field to become a `FrameCount` rather than the `u32` its sibling
carries; dropping it answers the same objection without adding a break, and the two variants then differ because the
record asks them to rather than by oversight.

**The phase gets its own newtype, and a first draft reused `QuantumOffset` instead.** Both are `0..Q`, and the
argument for sharing was that a phase *is* the offset at which a window's start sits in the zero-anchored grid. The
review refused it with a counterexample: anchor plan position 1 to sample 0, and plan position 63 renders at quantum
offset **62** while its phase is **63**. A quantum offset is where a sample sits in the render quantum carrying it,
which is an engine-timeline fact; a phase is a property of an anchoring of plan time. They coincide only for the
identity anchor, so one type for both would let a cross-timeline substitution type-check — the same objection that
earlier rejected a trait spanning `PlanPosition` and `SampleTime`, and it is right for the same reason.

`AnchorPhase::of` is total, for the argument `SampleTime::quantum_offset` already makes about the same cast: a
remainder modulo `Q` is below `Q`, and the compile-time assertion keeps `Q` inside `u16`. An earlier draft reached the
value through two fallible conversions with `unwrap_or(ZERO)` behind them, which would have reported phase zero for an
unreachable case — a wrong answer where proven arithmetic gives a right one.

**The oracle is the definition, not the algorithm.** The tests bucket the materialised periodic stream by absolute
quantum under a given phase — which is what the sliding window is an optimisation *of* — and check that the named
phase really overruns, and that an admitted loop overruns at none of the `Q` phases.

**The oracle was unsound twice, and reviews found both.** The first version materialised `Q / length + 4` passes,
which covers a window straddling one wrap and nothing else. Copy `n` sits `length * n` frames along, so against a fixed
grid its phase shifts by `length % Q` each copy and only returns after `Q / gcd(Q, length)` of them: with a
129-frame loop, events at 0 and 63 first share a phase-10 quantum at copy **10**, and the oracle answered "no
overrun" there. It now materialises a full alignment cycle, and that counterexample is a test of its own.

The second defect was in the bucketing. A frame before the phase's first boundary has a negative quantum index, and
`saturating_sub` folded every such frame into bucket zero **together with the first complete quantum**, which the
interior trim then discarded as an edge: a two-frame loop at phase 38 inspected no interior at all and answered "no
overrun" while `[38, 102)` held 32 events against a share of six. Shifting by one whole quantum instead of saturating
moves every index by exactly one and merges nothing. Both counterexamples are now tests of the oracle itself.

The implementation was never affected by either, and the reason is the distinction the first repair turns on: the
stream is periodic, so a window's content depends only on its start modulo `length` and `ceil(Q / length) + 2` copies
already cover every residue. The oracle asks a different question — *does **this** phase overrun* — and only that one
needs the cycle.

That test is weak on a wrap fixture, where two events one frame apart are together under 63 of the 64 phases, so a
second fixture has exactly **one** witnessing phase: two events `Q - 1` apart sit in one quantum only when a boundary
falls exactly on the first of them. That is what pins the named value rather than merely making it plausible, and it
is what catches a hardcoded phase.

**One mutation was attempted and is recorded as not a mutation.** Mapping the window start back into the loop interval
before taking its phase produces a *different* value — but wherever the start falls outside the interval, the two
events are less than `Q - 1` apart, so several phases witness and the mapped value is one of them. It is a valid
answer, not a wrong one. The scan's choice is therefore determinism rather than correctness, and the record says so
instead of implying a test holds it.

**What this does not do.** `admit_loop` had no caller when this slice landed: clause 4's other half — the state
change failing before activation with the prior transport state left in place — needs a loop activation operation,
which is transport activation and is the ADR named below. This slice completes the diagnostic contract, not the
wiring; the transport-activation slice below supplies it.

### Completed slice — transport activation, clauses 1 to 6

ADR-0050's clauses 1 to 6, built. **Clause 7's catch-up batch is deliberately not in this slice**, and the
superseded eleven-round attempt below says why. A seek is one value: `StreamControl::plan_activation` stamps a
candidate against a **working copy** of the minter with the schedule in force releasing its reservations into it,
`CompiledEventScheduler::offer` accepts or refuses it, and adoption at a quantum boundary swaps it in.

**The effective point is observed rather than reported.** The partition test seeks at a non-boundary time into a
position where a note is already sounding, so the frame the output changes on *is* the boundary — and it is the same
frame under `2048`, `256`, `64` and `37`-frame callbacks. A rule that activated at the start of the next render call
would have been simpler and would fail this immediately, which is what the check is for.

**Two things the code found that the record had only stated.**

- **The uniform placement shift is load-bearing, not commentary.** A candidate is stamped against the time it
  *requested*; adoption snaps that forward, and the first version left the placed events where they were — the very
  first partition run refused with `MissedEvent`, an event at 209 against a clock at 256. The shift is now an `O(1)`
  value applied at **every** read of the schedule, because applying it at some reads and not others would put the
  window and the events it selects on different timelines.
- **Adoption may not allocate, and the purity scan said so.** The first version built a separate retired value with
  `Box::new` on the audio thread. The two activation types are now **one**: the same box travels in as a candidate and
  back as the retired state, its vectors swapped with the scheduler's live ones, and the exchange slot is what says
  which role it is in.

**The mass release reaches the kernels the only way a gate can.** An envelope's gate is sample-positioned, so lowering
it means delivering a `TimedControl` at offset zero of the boundary quantum rather than writing node state. Those
gate-downs are **not events**, so they get their own preallocated buffer, sized by the identity partition, and
`timed_control_scratch_bytes` accounts for it — as does the queue they wait in, which a review found uncharged. The
*operation* is charged: `ProducerClass::Session` takes one unit at the boundary quantum, which is ADR-0046 clause 6's
"one operation, never one event per voice", and admission reserves it.

**A seek through a held note builds, which is the commonest seek there is.** The suffix is derived here from the
plan's whole admitted stream rather than supplied as one: a release whose note-on lies before the anchor has nothing
to pair with, so it is omitted and **counted**, off the audio thread, which ADR-0001 clause 16 requires of any
transformation that drops an event. A release the suffix can pair for itself is placed untouched, and one with no
note-on on either side is a malformed list and is refused.

**Adoption waits for a quantum.** A call served entirely from the carry renders none, and a window of no quanta has
no row for the release charge that adoption incurs. Waiting costs nothing observable and — unlike a first repair,
which skipped the whole activation while any debt was outstanding — nothing that depends on the partition either: a
call that renders no quantum writes no audio and does not move the clock, so the next call computes the same
boundary.

**One structural change to clause 6, amended in the record in the same transaction.** The control used to refuse to
*build* while a candidate existed. That is not what clause 6 says, and it is not evaluable either: `offer` is a
method on the schedule, which the audio thread owns, so the control sees the build and the collection and nothing
between them. The control now **issues** what a candidate supersedes, from the sequence it last promoted; a candidate
built between acceptance and collection therefore carries the superseded value necessarily and the offer refuses it.
Competing builds are allowed, which is what clause 6 wanted. The maintainer approved the amendment on 2026-08-27.

**Lateness is decided at the offer**, and the specification and ADR now both say which clock. Three implementations
disagreed with the contract before that was written down: `effective > requested` reports every off-grid request,
`requested < clock` at adoption reports the same ones because the clock stands on the boundary by then, and
`boundary > snap(requested)` misses a request whose own snap *is* the clock. Lateness and displacement are
independent and neither implies the other.

**A stream has one schedule.** Two schedulers are two exchanges, so one candidate could be accepted by each and —
adoption being infallible — adopted twice. A schedule is replaced by an activation; preparing another is refused.

**The minter stands still while a candidate holds a snapshot of it.** A stamping committed between a build and its
collection advances generations the snapshot has never seen, and promotion would roll them back, after which a later
note could be handed an identity that is already live. That is the price of the copy that makes an abandoned
candidate free.

**Eleven independent review rounds ran against this slice, finding nine, eight, four, four, five, three, two, four,
four, three and four defects; a twelfth, against the reduced slice, found two.** They are not enumerated
here — the durable ones are in the clauses above and
`SOUND-INV-018`'s conformance row — but three facts about them are worth keeping, because they are what shaped the
split below:

- **Six of the eleven `P1`s were the previous round's repair.** Rounds four, eight and ten each produced a fix that
  the next round found wrong: a deferral that broke partition invariance, an inherited transport state that made a
  duplicated exchange work rather than removing it, and a forced-low gate that silenced automation.
- **Every one of those collisions was in the same place** — the interaction between clause 7's catch-up history and
  clause 5's mass release, on the one control both of them move.
- **Two claims were withdrawn rather than defended**, and one test with them. A displacement overflow was argued
  unreachable on a duration rather than a bound; a cost bound was covered by a test that survived its own mutation.
  Both are now stated as untested with the reasoning attached.

Two further rounds ran against the reduced slice and found **five defects between them, one `P1`** — against four
`P1`s in the two rounds before the cut, which is the evidence it was made in the right place. The `P1` is the loop's
admission subject: an activation may enter a loop late, and the events it skips are replayed by every wrap, so the
subject is a pass anchored at the loop's **start** rather than the one the seek produces. Three were lifecycle: a
candidate built before the stream had a schedule to offer it to — and that no schedule could then be prepared for,
since an outstanding candidate is what holds the minter still; a pre-anchor history with no polyphony bound of its
own, so a timeline the producer was never entitled to emit could still decide which crossing releases the suffix
omits; and a test total of this section's own that did not add up. All are repaired.

A third and fourth round against the reduced slice found three more, none of them `P1`: the loop-admission subject
left a stream-sized buffer nothing read; `offer` gained a **fourth** refusal cause that `SOUND-INV-018`, ADR-0050 and
the counter's own documentation all still said was three — and a fifth the first correction still missed, the
pairing refusal, which is also the one that deliberately increments no counter, because the counters belong to the
stream that was offered to and that refusal has no such stream. That enumeration was corrected in one place and
then another before the sweep reached zero, which is the failure mode `PROCESS.md` names: propagate a decision to
every occurrence, not to the one the review pointed at; and the
reduction left claims about a catch-up batch in code that no longer has one. All three are what the stopping rule
calls a false claim, and all three are repaired.

**One is not repaired and is recorded instead.** Dropping a candidate rather than withdrawing it strands the
outstanding count, and every later `stamp_compiled` then refuses. The value cannot reach its control from `Drop`, and
the exposure is small — after preparation there is no legitimate second stamping, because a stream has one schedule —
but it is a caller obligation the type system does not enforce, and the next slice to touch this ownership should
carry a token that does.

Forty-nine named checks: forty-four in `transport_activation.rs`, four in `transport.rs`, and the control-scratch
budget's. Every behavioural repair is **mutation-verified**: the repair reverted and the test
that covers it observed to fail with the symptom the review named, including several that fail as audible output
rather than as an error.

**What is not built, and is named rather than left to be found:**

- **Offer and collect still reach the audio thread's values directly.** Building a candidate no longer borrows the
  schedule, which is the half of clause 9 that was broken — but handing the candidate over and taking the retired
  value back are still `&mut` calls. A real hand-off needs a lock-free single-slot mailbox, which needs either
  `unsafe` or a dependency this crate's boundary test fixes; both need the maintainer's approval.
- **The loop is recorded rather than enforced.** `admit_loop` runs when the candidate is built — against the pass a
  wrap would produce, which took two reviews to get right — and adoption puts the interval in force. But no wrap is
  implemented, so nothing repeats it and the schedule is not bounded by its end: an event past that end plays and
  reserves an identity where wrapping would make it unreachable. **And that admission is a density check only.** It
  compares the repeating pass's events against the compiled share and does not check that pass's polyphony, so a
  loop entered after one note opened can be recorded and still over-emit at its first real wrap. An independent
  review raised the loop three times across three shapes of the same code; the maintainer chose on 2026-08-28 to
  declare the boundary rather than decide, in code, a behaviour no ADR clause describes. **The polyphony half is
  since built** — see the slice below — and the enforcement is the wrap's.
- **The tempo map is not in the activation.** Clause 3 puts it in the atomic set and it belongs there, but
  `SessionScheduler` owns the only one that exists and nothing replaces one during playback yet.
- **Clause 8's obligations are untouched**, as the record says they are: a non-compiled producer's holds have no
  redemption authority, and the control must remain the only minter while an activation is outstanding. ADR-0051
  raised the count to **three** — one scalar gate reached by two producers has no ownership law — and that third one
  is untouched here for the same reason: it binds live ingress, which this stream does not have.
- **Two branches are defensive and untested**, with their arguments in the code rather than as reassurances: a
  displacement that leaves engine time, and an outstanding occurrence that does not resolve as live.

### Completed slice — the locate catch-up

ADR-0051, built. The batch carries one row per prepared target at the requested time, is charged to the session share
beside clause 5's release operation, and is published **after** the boundary gate-downs and **before** the new
stream's own events at that sample. `NodeState::control_value` is the symmetric reader of `set_control` that supplies
a target with no history its prepared value, and it answers for the gate that `set_control` deliberately ignores.

**Scope held to what this phase reaches, deliberately.** Two conditions the record states as general are established
**structurally** rather than as branches: the release scope, because the compiled producer is the only one that
emits, and alias aggregation, which comes free because the substitution set is keyed on the physical
`(node, control)` pair rather than on a slot. Neither is a dead branch and neither has a test that could only pass
vacuously. Clause 6 is what keeps the first true, and it is live ingress's entry cost rather than this slice's.

**One correction the code forced on the record**, made rather than left to a later reader: an earlier draft said the
preserved gate-down adds to what admission must check. It does not. The write takes the omitted release's place one
for one at its own position, so the candidate never carries more events at a position than the admitted stream did,
and that stream was already judged against the compiled share. The draft also promised a compile-time refusal for a
note slot whose gate has no prepared row; it is an activation-build refusal, which is where the value is needed and
the first point that can name the offending event.

**What it does not do is refuse an oversized catch-up at admission**, and an independent review of this update
found a draft of it claiming otherwise. The plan's session request is computed and reported — the prepared-target
count plus one for clause 5's boundary mass release — but `SessionEventShare` is excluded from
`ResourceField::is_admission_checked` and is not advisory, so a row that exceeds the share yields neither a refusal
nor a warning. A plan with more prepared targets than the share compiles, and the overrun arrives at the first locate
as the publication fault that ends the stream. That is ADR-0046 clause 3's own case, an admitted plan reaching a
runtime miss; `is_admission_checked`'s documentation already names the plan-dependent admission of all five remaining
shares as later Phase 3 work, and this is one of them.

**One repair the merge review forced, in the loop rather than in the catch-up.** Clause 5's preserved gate write and
the loop's admission were built in different slices and disagreed: `repeating_pass` skipped a crossing release's
position, which was right while the omission dropped the whole event and wrong once it kept the gate write. The
repeating pass was undercounted by one per crossing release, so an interval whose wrap would overrun the compiled
share was admitted — ADR-0046 clause 3's case again, an admitted plan reaching a runtime miss. The position now
counts, because the density is the event's question and not the note contract's. Mutation-verified: the skip
restored, the interval is admitted where it must be refused, at ninety-seven against ninety-six.

**And the repair collapsed the thing it repaired**, which a second merge-review round is what established. Once every
event inside the interval carries an event, the open-depth bookkeeping decides nothing: `repeating_pass` is the
interval's positions and no more. The state is removed rather than left as a check that cannot fail.

That also removed the old test's subject twice over. It distinguished "the original stream" from "the list the
candidate carries" **by** the crossing release, which no longer differs between them — and the fallback the first
repair reached for, the history before the loop's start, is not a discriminator either: `admit_loop` filters its own
input to the interval, so history cannot reach the window whichever list it is handed. That first rebuild was a test
that could not fail, and the review caught it.

The subject that does survive is the **suffix**. An activation entering a loop late carries events from its own
position, and every wrap after it replays the ones before that. The test is rebuilt there: ninety-six writes at 110
and one at 290, requested at 260, so the suffix carries only the write at 290 while the pass that repeats puts the
ninety-six at 310 — twenty frames away, ninety-seven in one window. Mutation-verified against judging the suffix.
The old construction becomes the regression test for the undercount, mutation-verified against skipping a release's
position.

The decisive test is a comparison rather than a threshold, and it carries two falsifiers because they catch different
wrong implementations. Two streams differ only by a gate write that is **inert while playing through** — the kernel
returns early on a re-asserted level — so their seeked output must be identical; the equality is what rejects the
abandoned design's last-write restore. And both must be silent; that is what rejects dropping the substitution
altogether, which would leave them equal and both wrong. Mutation-verified: disabling the substitution leaves the
equality passing and fails the silence.

### Superseded: the eleven-round attempt

**This is the record of the attempt the slice above replaced**, kept because it is why ADR-0051 exists rather than
because anything in it is pending. Both attempts were after the same obligation: a locate should restore every
prepared target to the value in force at its destination, or seeking past a parameter change leaves that parameter
where the pre-seek position left it. The slice above discharges it, under the gate exception ADR-0051 decides; what
follows is why the first attempt could not.

**Why it became its own slice.** A working implementation existed and was reviewed eleven times as part of the
transport-activation slice above it. The findings did not converge: they settled at three or four a round, and the
same interaction produced them each time — the batch and clause 5's mass release both move an envelope's **gate**,
and the rules that reconcile them accumulated one review at a time until four of them sat in one function. Rounds
six, ten and eleven each found a `P1` there, and each `P1` was the previous round's repair for the same code:

- a note edge that did not write the history at all, so automation that a later note-off had undone was restored;
- a crossing note forced low unconditionally, which silenced automation that came *after* its note-on;
- a pairing counter reused as the anchor depth, so omitting a crossing release erased the record that the note had
  been open — and the seek re-opened a gate nothing would ever release.

None of the three is subtle in isolation. Together they are a design that was never written down, which is what
`PROCESS.md`'s decision-timing rule exists to prevent.

**That design is now written down, and the question is answered.** *What does a locate owe a control that both
automation and a note contract can move?* — settled by a design consultation before any code, and approved by the
maintainer on 2026-08-28. [ADR-0051](decisions/ADR-0051-locate-catch-up-gate-exception.md) is the successor record
that carries the rule and its audible consequence — `PROCESS.md` takes an accepted decision through a successor
rather than an in-place rewrite, and an independent review found the first draft had rewritten ADR-0050 instead.
Only the shape is repeated here:

> The catch-up computes the last pre-destination write for every prepared target, note edges included, and then
> substitutes `ZERO` for every physical `(node, control)` gate held open by an in-scope note contract immediately
> before the destination.

Three things about the answer are worth keeping, because each contradicts what the eleven rounds built:

- **The last-write rule is not semantics-preserving for an edge-triggered control.** Automation raising a gate a note
  already holds is inert while playing through — the kernel returns early when the level is re-asserted. Restoring it
  after clause 5's mass release lowered the gate is *not* inert: release-then-batch at one offset is a rising edge,
  so the envelope re-attacks with no note contract behind it. That is note chasing, which clause 5 declines.
- **The predicate is the destination-open contract, not the release scope**, and they are different sets. A forward
  seek can land inside a note the retired stream never sounded, and that gate must still be low.
- **The substitution aggregates by physical target.** Parameter aliases onto one gate would otherwise disagree, and
  the last row published would win.

`written_by_note` — the one piece of state that produced six of the eleven `P1`s, all in this interaction — does not
exist in the answer. It is removed rather than reconciled.

**A second decision was needed, and a second review round is what found it.** ADR-0050 clause 5's omission of a
crossing release drops the **gate-down the plan authored** along with the note contract, so automation raising that
gate at or after the destination leaves it high with nothing left to lower it — a stuck note, where playing through
ends it at the release. ADR-0051 clause 5 keeps the gate write and omits only the identity: a bare `SetParameter` of
`ZERO` at the release's own position. The maintainer approved it on 2026-08-28, put separately from option B. Two
consequences were stated with it, and the build corrected the first: the suffix gains **one event per omitted
release**, but that adds nothing for admission to check, because the write takes the omitted release's place one for
one at its own position; and a note target whose gate is not a prepared parameter target is refused when the
activation is built, which is where the value is needed.

**The reviewer for this record is not Codex.** Codex was the design consultant — it selected the option and supplied
the predicate, the aggregation rule and both obligations — so `AGENTS.md`'s independence property disqualifies it
from reviewing constraints it authored. Its own second read raised that, and crediting it with both roles was a real
process defect in the first draft rather than a wording problem.

**The record's waiver is discharged.** The draft that carried it concluded no qualifying reader existed, because the
Gemini CLI cannot authenticate — and missed the replacement its own error message names. The Antigravity CLI (`agy`)
runs Gemini, did not author this change, and is not the author's family, so it satisfies both properties. Its read
found the `HOST-INV-022` contradiction repaired above, and bounded clause 3's aggregation claim. **A dead tool is not
the same fact as a dead family**, and treating them as one is what put a waiver on a record that did not need it.

The work is preserved on the local branch `wip/transport-activation-full` at `77c08eba`, which carries the full
implementation, its tests, and the eleven rounds of repair. **It is superseded rather than resumed**: it took the
unamended clause 7 literally, which is the case the amendment now decides the other way. It is not merged and is not
a candidate for merging; it is there so this attempt started from evidence rather than from memory.

The same consultation found a third obligation for ADR-0050 clause 8, now recorded in ADR-0051 clause 5 and in
`SOUND-INV-018`: **a gate reached by more than one
producer cannot be released selectively.** A compiled note and a surviving live note can address one scalar gate, and
ending the compiled one writes `ZERO` to both. The scope predicate alone does not close it, so live ingress needs
target sharing refused at admission, producer-exclusive gates, or an ownership law — decided before a second producer
may emit onto one gate.

**The three things it also needed are built, in the slice above.** `ResourceField::SessionEventShare` now carries
the plan's prepared-target count plus one for clause 5's boundary mass release as its requested amount, against the
profile's share as the available one. `HOST-INV-022`'s bound is the batch's size, which ADR-0051 leaves unchanged.
And `NodeState` has the symmetric reader of `set_control` that was built and removed with the rest. Reporting that
request is not refusing it, which the slice above records.

### Completed slice — the stream's two owners

ADR-0050 clause 9, plus the audio-thread half of clause 5 that the split is what makes reachable.
`StreamControl` owns the epoch, the plan, the anchor and the identity **minter**;
`PreparedRenderer` keeps the clock, the carries, node state, the live-note registry and adoption. `StreamControl::open`
is the only constructor for either half: it issues the epoch, builds the minter, and prepares the renderer against
**that** table's identity, so the pair it returns always answers to itself. Two identities there would make every one
of the stream's own events look foreign to its own registry.

**A caller can still cross two streams' halves, and the first version of this note claimed otherwise.** Two halves are
two values; nothing in the type system pairs them. The crossing is refused where it becomes wrong — a schedule carries
the epoch of the control that stamped it, and `render` refuses a renderer whose epoch differs, rejecting the whole
schedule rather than silently discarding each of its events as stale. A named test drives exactly that pairing.

**This is a split of what already existed.** The renderer documented its minter as "off the audio thread" and its
registry as "the audio thread's half" while holding both, and nothing on the audio thread ever read the minter.

**The control holds the plan too, and that is what makes the split real rather than nominal.** A first version left
`CompiledEventScheduler::prepare` taking `&PreparedRenderer` to read the plan and the epoch; an independent review
established that an off-thread builder cannot hold a shared borrow of a value the audio thread mutates, so the
ownership would have been split in name only. Preparation now touches no audio-thread state at all. The plan is
shared through one `Arc` rather than copied, because a second copy would be memory admission never accounted for.

**One check was removed rather than kept untested.** `SchedulePrepareError::AnchorMismatch` refused a placement the
renderer was not anchored at. With one owner for the anchor there is nothing for a caller to supply and neither half
has a setter, so the disagreement is now unrepresentable — and a rule no test can reach is a rule nobody has checked.
The activation slice is what makes drift possible again, and it brings the guard back with a test that can reach it.

`LiveNotes` gained the scoped mass release clause 5 puts on the audio thread, and the producer ranges a scope names.
Both identity halves now build their spans through one `producer_spans`, because two loops computing one partition is
how they come to disagree — and here they must agree exactly, since the minter allocates inside a producer's span and
the registry clears inside the same one.

The registry's release is **all or nothing**, and the first version was not. A caller passes storage for the nodes so
it can lower those gates without a second walk; a buffer too short to name the scope now ends nothing rather than
clearing entries it cannot report. An independent review found the difference: a cleared-but-unnamed note has no
registry entry and no reported node, so nothing can release it and nothing can lower its gate — it sounds forever.
Refusing leaves every note reachable. The bound is the scope's **span** rather than its live count, because a span is
what a caller can size against an admitted declaration, so the refusal is unreachable for a conforming caller.

Both mass releases now return `HeldNoteCount` rather than a raw `u32`; `live()` and `retired()` keep their primitives
as pre-existing surface this slice does not touch. Two properties are mutation-verified: clearing the whole registry
instead of the producer's span, and letting the count follow the buffer length.

`identity_bytes` charges **two** range tables rather than one, and both halves now report their own
`storage_bytes`. The registry gained a copy of the spans in this slice, and a budget that kept charging one would have
reported a ceiling preparation then allocates past — which is admission passing a plan it should refuse.

The second `storage_bytes` exists so the budget can be **checked against what the halves hold** rather than against a
restatement of its own formula. The existing scratch-budget test varies polyphony, which moves the per-index term and
cannot see the per-producer one at all; the new test varies the producer count independently of the index count, and
it is mutation-verified against charging one table.

`Option::take` joins the render loop's purity allowlist with its justification: a move out plus a `None` write, on
storage the caller already holds.

**API breaks, inside the approved `synth_engine_v2` scope**: `PreparedRenderer::prepare` is crate-private;
`CompiledEventScheduler::prepare` takes `(&mut StreamControl, &AdmittedCompiledStream)`; `stamp_compiled` takes
`(&mut StreamControl, &[CompiledEvent])`; `IdentityTable::release_all` returns `HeldNoteCount`;
`PreparedRenderer::{plan, added_latency}` are no longer `const fn`, because the plan is behind an `Arc`; and
`SchedulePrepareError::AnchorMismatch` is removed, which is a break for an exhaustive match. New surface:
`stream::StreamControl` and `LiveNotes::release_all`.

### Accepted — ADR-0050 transport activation, with its specification transaction

[ADR-0050](decisions/ADR-0050-transport-activation.md) is `Accepted`. It is the decision record the blocked
re-anchoring slice named as its next step, and it settles **six** things together rather than the five that slice
listed: the sixth is the locate catch-up, which the blocking review found missing from the design entirely.

The five findings that blocked the slice are each answered rather than worked around, and the answers are the record's
clauses. Two of them turned into structure rather than rules:

- **A quantum-granular activation point is what makes the junction admissible.** Shares are charged per destination
  quantum, so a boundary-aligned activation puts the old stream's last events and the new stream's first events in
  different quanta — no quantum mixes the two, and the junction needs no third admission rule. A sample-exact
  activation would need a junction check shaped like ADR-0046 clause 4's periodic loop extension. That is an argument
  *for* the granularity beyond implementability, and it was not in the design the review blocked.
- **Ending a crossing note and reclaiming the retired schedule's minter index are one act.** A leftover note-on is
  precisely a minted index whose note was never released, so ADR-0046 clause 6's bounded mass release — scoped to the
  replaced producers, charged to the session share — closes the identity leak the review found. The `Drop` the crate
  does not have could not have run on the audio thread anyway.

An independent design consultation was taken **before** the record was written, and four of its findings are in the
accepted text rather than left for a reviewer to rediscover:

- **A repeating wrap must keep its ideal phase.** A loop length that is not a whole number of quanta snaps at every
  wrap; deriving the `k`-th wrap's requested time from the previous wrap's *effective* point accumulates those
  roundings and makes the loop permanently longer than the one the user set. Deriving it from the ideal timeline
  keeps each error independent and under one quantum. The two are indistinguishable for a loop length that happens to
  be a multiple of `Q`, which is why the rule is written down.
- **A compiled producer has no hold to redeem.** A first draft had the boundary mass release "redeeming every
  affected hold", which misstates ADR-0046 clause 6: compiled releases use plan entitlements. Ending a compiled note
  frees an identity and nothing else.
- **The mass release reaches both halves of the identity state**, and only one half has the operation today.
  `IdentityTable::release_all` takes a scope; `LiveNotes` has no counterpart and does not carry the producer ranges a
  scope names. An entry left in the registry after its index was freed is resolvable by an occurrence the minter has
  since reissued.
- **The heard cost is larger than the placement error.** Up to `Q - 1` frames of placement plus the stream's existing
  `Q` of output latency is 127 frames, about 2.65 ms at 48 kHz. Quoting only the 1.33 ms would have understated what
  a user experiences by half.

**Sample-exact seek and loop are explicitly not claimed**, in the ADR, in `SOUND-INV-018` and in this note. Placement
error is up to `Q - 1` frames. The master plan's Part III requirement is untouched by this record, and no gate may be
read as closing it on this record's strength.

Acceptance updated two current specifications in the same transaction:

- **`SOUND-INV-018` is new**: the activation value, its effective point, the kept carry, the atomic state set and the
  three refusals that happen at the **offer** rather than at adoption, the mass release at the boundary, the
  sequence-based freshness rule, the catch-up batch, and the two owners.
- **`HOST-INV-022` is new**: the activation exchange as a fixed single-slot **non-dropping** session store — it takes
  no row in the renderer-ingress registry, because that table registers live input that may drop — and the catch-up
  batch as the bounded quantity `HOST-INV-021`'s session check compares against.

**An independent semantic review then found seven defects, five of them `P1`, and each is repaired in the accepted
text rather than deferred.** They are recorded because four of them are traps a later implementer would otherwise
walk into again:

- **The candidate is stamped against a copy of the minter, and it took three attempts to get there.** The first
  draft said an abandoned candidate "consumes nothing", which is false: `stamp_compiled` commits its working minter on
  success. The second made withdrawal release the candidate's outstanding set — also insufficient, because stamping
  is *not* reversible by releasing: a note-on paired inside the list already advanced its index's generation and may
  have retired it, so a fully paired candidate has an empty outstanding set while having spent generations. It also
  left the outgoing schedule's reservations occupying the shared producer range, so a producer whose declared
  polyphony the outgoing schedule already used could not build **any** replacement. The copy closes all three:
  the control releases the outgoing schedule's outstanding occurrences into a working copy, stamps the candidate
  against it, drops the copy on withdrawal, and promotes it the moment an offer is accepted — which is safe precisely
  because adoption is infallible.
- **The registry's range-scoped clear is safe, but not for the reason the repair first gave.** An occurrence does not
  enter `LiveNotes` when its event is applied; the renderer admits occurrences during *resolution*, before the call's
  first quantum renders. The true reason is that the candidate's events reach no render call at all until after
  adoption.
- **An ordinary seek into a sounding compiled note was unbuildable.** The suffix holds a release whose note-on lies
  before the anchor, and stamping refuses such a list. A suffix now omits that release and counts the omission —
  off-thread, so it is a named transformation rather than a renderer-side drop. Note chasing is explicitly not done.
- **The catch-up batch has to cover every prepared target, not only the automated ones.** A control value survives an
  activation in node state, so seeking to before a parameter's first automation point would otherwise leave the value
  that automation set. A target with no preceding event now carries its prepared value — which also makes the batch's
  size exactly the prepared-target count, so admission has one number to compare rather than a worst case to search
  for.
- **The return slot has to carry every heap-owning piece.** A `TempoMap` owns a `Vec`; a replacement that returned
  only the anchor, schedule and loop would free that allocation on the audio thread while the record claimed the
  exchange was real-time safe. The slot is also occupied in two distinguishable ways — pending candidate, or
  uncollected retired value — and only the second means the off-thread half fell behind.

**A third round found four more `P1`s, and it is why the record now carries an explicit scope and a clause naming
what it does not close.** All four traced to one root: the identity and hold halves of the contract were being
extended across boundaries whose producers do not exist. Three repairs and two withdrawals came out of it:

- **Promotion moved from offer-acceptance to collection.** "Adoption is infallible" does not mean "adoption is
  reached": the renderer can end the epoch first — oversized callback, publication fault, clock exhaustion — and then
  no later call advances toward the boundary. Promoting at acceptance would have spent the candidate's generations
  for an activation that never happened. Collection is the first moment adoption is a fact. The window that opens is
  closed by a rule rather than by compensation: the control builds no second candidate while one is outstanding.
- **The recorded outstanding set is *not* the set the boundary ends**, and an earlier repair said it was. They
  overlap without containing each other — a note paired later in the outgoing list is sounding at the boundary but
  already freed its index at stamping, while an unpaired note-on beyond the boundary is held but never sounded. Each
  half deals with its own set, which is the same reason `LiveNotes` exists at all.
- **The ADR-0047 clause 7 reconciliation is narrower than claimed.** The orphan behaviour is the registry's. The
  allocator's generation advance is not a second delivery of it, and the protection against a stale identity matching
  a new note comes from SOUND-INV-017's never-reused generation rather than from any release's timing.
- **Withdrawn: extending the contract to a non-compiled producer.** ADR-0046 clause 6 wants every affected hold
  redeemed atomically at application, and there is no hold store to redeem from. A schedule replacement whose
  producers include a non-compiled one is out of scope until holds exist.
- **Withdrawn: tolerating a second minter.** A live note-on minting into the authoritative table while a candidate's
  copy is outstanding would be erased by promotion, winding a generation backwards. While an activation is
  outstanding the control is the only minter — true by construction today, and an entry cost for live ingress.

Those two withdrawals are the record's clause 8, and `NOW.md` carries them against the slices that own them.

**A fourth round confirmed no unfillable contract hole for a compiled-producer stream**, and produced one
reclassification and three consistency repairs. The reclassification is the substantive one: the relationship to
ADR-0047 clause 7 is an **amendment**, not a reconciliation. Two drafts argued the records already agreed; they do
not, because clause 7 says the generation advance happens *at application* and here it happens at stamping, at build,
and at collection — none of them application. What the amendment preserves is what clause 7 is for, the orphan, which
the registry delivers at the boundary. `SOUND-INV-017` now records the amendment rather than leaving the two
specifications to disagree.

The three consistency repairs: a stale sentence still promoting the copy at offer-acceptance; a scope paragraph
pointing at clause 9 where the obligations are clause 8; and **"outstanding"** left undefined, which appeared to
forbid the two competing builds clause 6 deliberately allows. It means *accepted at the offer and not yet
collected* — building competing candidates is fine until one is accepted.

One more false claim was caught there and is worth keeping, because it is the kind that reads as harmless: the record
said plans "today declare only compiled note producers". They do not — `NoteProducerDeclaration` carries a `compiled`
flag and a plan may declare a non-compiled producer. The true and weaker fact is that nothing non-compiled can yet
**emit**, so the compiled producer is the only one that mints, and that is what the scope rests on.

A repair that introduces a new defect is still a defect, and the count here is the honest one: **seven findings, then
four, then four, then four**, across one design consultation and four reviews. The identity design is a working
copy rather than a reclaim because of round two; it is promoted at collection rather than at acceptance, and bounded
to compiled producers, because of round three.

The two `P2` findings from the first round were an over-claim and a contradiction: EVD-0017's figure is a **floor**
measured over the compiled share alone, so it cannot establish the eventual two-pass cost, and one paragraph still
said a discarded carry could lose `maximum_block_size + Q` frames after the neighbouring one had established that
between calls it is at most `Q`.

Two consequences are worth stating because they are obligations rather than descriptions:

- **The minter moves out of `PreparedRenderer`.** Clause 9 gives the stream two owners, and the minter is the
  off-thread half's; keeping it inside the renderer is what makes a candidate impossible to build while the stream
  renders. The struct already documents `minter` as "off the audio thread" and `live_notes` as "the audio thread's
  half", so this is a split of what exists rather than a new structure — but it breaks
  `CompiledEventScheduler::prepare` and `stamp_compiled`, and that break is asked for before it is made.
- **`SOUND-INV-018` has no coverage yet**, and its conformance row says so rather than being omitted. The record
  precedes the slice deliberately: the five findings are questions, and code written against an unanswered one is what
  the record exists to prevent.

### Completed slice — the repeating pass's polyphony

The first half of the wrap debt the transport-activation slice left, and deliberately only the first half.
`plan_activation` now judges the pass a wrap would replay against **two** bounds rather than one: ADR-0046 clause 4's
periodic extension against the compiled share, which it already did, and `SOUND-INV-017`'s producer range against the
notes that pass holds open at one instant, which nothing did.

**The second bound is not clause 4's, and calling it that would have been the false claim this slice nearly shipped.**
Clause 4 says a wrap "cannot fail for compiled **capacity**", and capacity is the share. What a pass can also exhaust
is identity, and the rule bounding that is `SOUND-INV-017`'s admitted range — already enforced on the history the
anchored walk sees and on the suffix `stamp_into` mints. The pass a wrap replays is a **third timeline the same
producer emits**, and it had no enforcement point at all.

**Its subject is unreachable from either existing check, which is what makes the gap real rather than theoretical.**
A pass exceeds while both existing bounds hold only when one note opens before the activation's destination and
another after it: the first is history, the second is the suffix, neither sees two, and the pass a wrap replays holds
both. That is the decisive test's construction, and it is exact — one note at `Q`, one at `5Q`, a destination at `4Q`,
and a producer admitted for one.

**Both quantities come from one walk**, because they are properties of the same events and two walks over one interval
is how they come to disagree about which events those are. `repeating_pass` returns them together for the reason
`producer_spans` is one function.

Two rules the walk applies were settled before it was written, both from ADR-0050 and ADR-0051 rather than invented
here. The pass starts with **nothing sounding**, because clause 5's boundary mass release ends what the previous pass
opened; inheriting the depth at `loop_start` would charge every note once where it opens and again in every later
pass. And a release whose on edge lies before the interval is ADR-0051 clause 5's **crossing release** — a bare
gate-down carrying no note contract — so it lowers nothing.

**A single-slot plan cannot exhibit that second rule**, and the test says so rather than asserting it vacuously: a
release takes the most recent unclosed on edge for its own slot, so the crossing branch is reachable only when that
slot holds nothing, and then the count it would lower is zero. It needs two note slots — and two admitted notes, since
with one the crossing note is open beside every pass note that opens before the destination and the history's own
bound refuses the stream first.

**Pairing is left to its owners.** A release matching nothing on either side raises the peak rather than refusing
here: the anchored walk refuses one in history and `stamp_into` refuses one in the suffix, and a third authority is
how the three come to disagree. Leaving it unpaired can only leave the count higher than the truth, so a malformed
stream is refused by this check or by its owner and never admitted by both.

**This narrows delivered behaviour, and that is the point rather than a side effect.** A loop whose pass over-emits
could be recorded before this slice and worked, because nothing replays it. It is now refused at the build, which is
what ADR-0050 clause 3 means by wanting the interval already admitted when it joins the atomic set.

Six named checks — four in `transport_activation.rs`, two beside `admit.rs` — and every behavioural claim is
mutation-verified against its own falsifier: neutralising the check fails the two refusal tests and neither of the
others, seeding the count from the depth at `loop_start` fails only the zero-start test, letting a crossing release
decrement fails only the crossing test, and removing the no-producer guard fails only the test that names it.

**Two boundary defects the independent reads found, both repaired.** Neither touches the peak; both are about what
the new path says when its subject does not exist:

- **A plan declaring no compiled note producer was reported as one admitting zero notes.** Those are different facts,
  and because the comparison runs before stamping it classified one invalid note two ways depending on whether a loop
  interval was supplied. The comparison is now skipped where there is no producer, and `require_note_producer` and
  `stamp_into` keep the refusal that is theirs.
- **`admit_loop_polyphony` took two positions and accepted an inverted pair**, where `admit_loop` answers `EmptyLoop`
  for one. It takes a `LoopInterval` instead, so the case is removed rather than adjudicated — an `EmptyLoop` branch
  here would be a rule no caller could reach, which is a rule nobody has checked. The asymmetry with `admit_loop` is
  recorded at the function rather than left to look like an oversight.

One repair beside it: a doc comment merged into `gate_rows` at the locate catch-up's merge, carrying `repeating_pass`'
own heading with it.

**API break, approved by the maintainer on 2026-08-28.** `AdmissionError` gains `LoopPolyphonyOverProducer`, which
breaks an exhaustive match because the enum is not `#[non_exhaustive]`. It was asked for separately rather than
carried by an earlier approval, and the first draft of this note claimed otherwise: no standing licence covers it, the
section above says in as many words that a later variant asks again, and the same enum's previous variant
`LoopWindowOverShare` was itself approved on its own on 2026-08-26. An independent review found the false claim. The
grounds are the ones every approval here has rested on — the crate is experimental, is not a dependency of the
workspace's default members, and `crate_boundary.rs` permits one in-repo consumer, `pertylizer` as a dev-dependency
for the three measurement examples. Making the enum `#[non_exhaustive]` was offered a third time and declined a third
time, so the next variant asks again. New surface: `admit::admit_loop_polyphony`.

**What this does not do is implement the wrap**, and the reason is a decision rather than an estimate. A design
consultation taken before any code established that a wrap replays a list whose occurrences were minted **once**, off
the audio thread, and that replaying them is not the small amendment to `SOUND-INV-017` it looks like: it converts a
stale-event defect into a wrong-note action, and `LiveNotes::admit` overwrites on an equal generation so the registry
supplies no second defence. The consultation also refuted a generation displacement returned through `RetiredState`
— a candidate is stamped before the wraps it must be ahead of — and an off-thread pre-built pass, which contradicts
clause 4's accepted "once accepted, a wrap cannot fail". It left nine questions the accepted records do not answer,
among them the first ideal-wrap origin after a late activation, precedence between a wrap and a pending activation,
and whether several ideal wraps snapping to one boundary coalesce.

**ADR-0052 owes those answers before the wrap is built.** The maintainer chose the split on 2026-08-28, and the
identity mechanism is left open for that record rather than pre-committed here.

## Second active stream: Phase 0B

Outcome: complete the V1 migration inventories and the durable Project and Application Core contracts required before
Phase 10.

**Resumed on 2026-08-29** at the maintainer's request; nothing was holding it. One task is selected at a time, as this
section required at the pause.

### Selected task — P00B-T001, the persisted-state ownership audit

Observable completion check, copied from the master plan's first Phase 0B exit bullet and the register vocabulary:

- every currently persisted field appears **exactly once** in the
  [state-ownership ledger](inventories/state-ownership.md) with a proposed V2 owner or an explicit removal decision;
- every owner is supported by a named consumer in the code or by a recorded maintainer product choice; and
- the ledger's own status rule is satisfied, so entries reach `Classified` only once an `EVD` record carries the
  audit's coverage claim.

**All three are met, and P00B-T001 is `Complete`.** The ledger holds 64 entries, every one `Classified`, and the
coverage check that the documentation gate runs is what keeps the first and third bullets true rather than asserted.

### Completed slice — the eleven contested owners

Every `Intended V2 owner` cell the audit had left blank is now filled. The blanks were not arbitrary: they are the
cases the master plan names as borderline, which "must not remain in a project merely because the current save path
can reach them".

**Seven were settled by tracing the consumer rather than by choosing.** Two of those traces **refuted a claim an
earlier pass had recorded**, and in both the wrong claim was what kept the cell blank:

- **`global.glide_time` is not the master plan's "preview glide"** (STATE-0009). Project load sends it to the engine
  and a voice falls back to it at every note start when the note carries no glide of its own, so it is audible on
  sequenced playback. It is authored project data.
- **`patch.settings.octave_offset` is not a duplicate of the keyboard octave** (STATE-0035's "duplicates of
  STATE-0007..0009"). It is a per-patch field that the standalone-patch path mirrors through the GUI keyboard in
  **both** directions, and that a separate engine map carries to the preview path; the keyboard octave is that
  widget's own base note on the project path, which reaches the patch field's mirror on neither side. It is split
  out as STATE-0062, which also records what the mirror costs: the keyboard holds one value while the field is per
  patch, so loading a second instrument's patch overwrites what the first one set.

**Four are product choices and the maintainer decided them on 2026-08-29**: `active_instrument_id` stays in the
document as editor metadata; the keyboard octave becomes user settings; `solo` becomes runtime session for
instruments, tracks and return buses alike; and the transport loop stays in the document. The ledger's
*Contested-case decisions* section is the register for these and is not repeated here. **Two of them break delivered
behavior** — a V1 project's keyboard octave and its solo states would not come back — and the ledger records that
against each entry, because **ADR-0013** carries the breaks when it is drafted. Three of the four are named in the
master plan's list for that record and `solo` is not, but the list is not closed and `solo` is the same boundary
question; the ledger records that rather than claiming the list already covers it. ADR-0018 governs which layout
and organization data is shared project content, not where a contested field belongs.

**No ADR was drafted, and that is the decision-timing rule rather than an omission.** The exit gate asks for a
*proposed* owner, no implementation slice depends on the classification before Phase 10A, and `PROCESS.md` times a
decision by its first dependent slice rather than by a register entry's phase label. The ledger names ADR-0013 and
ADR-0018 as the records that will make the classification durable.

**Splitting is what keeps "exactly once" true.** `solo` and `patch.settings.octave_offset` no longer share an owner
with the fields they were bundled with, so four entries were added (STATE-0061 to STATE-0064) and the rows they came
from had those fields removed from their own field lists. The ledger holds 64 entries; next free is `STATE-0065`.

**An independent review found four defects and all four were repaired**, three of them factual rather than
editorial. The octave mirror above was recorded in one direction when it runs in two. `pattern.next_note_id` was
enumerated under STATE-0040 *and* STATE-0046, which made the exactly-once claim in this slice's completion check
false. STATE-0046 was classified `Removed` against ADR-0014, which **replaces** the seven per-kind cursors with
one validated `AllocationRecord` and refuses to derive the next ordinal from surviving content precisely because
that reissues a deleted entity's ordinal — so persisted allocation state stays in the document and the cell is
`Project document`. That one was avoidable: the coupled record already answered the question and was not read
before the cell was written.

Method limit, recorded in the ledger's audit-pass row: consumers were **read, not executed**, so no cell is yet
verified by a round-trip fixture. That is P00B-T005.

### Completed slice — EVD-0018, the coverage claim made falsifiable

The ledger has asserted since its first pass that every persisted field appears exactly once, and asserted it from a
count. The inventory rules say in as many words that a matching count is not coverage, and a count cannot tell a field
nobody claimed from a field two entries claim — which is precisely the defect the previous slice's review found in
`pattern.next_note_id`, by reading rather than by counting.

[EVD-0018](evidence/phase-00b/EVD-0018-state-ownership-coverage.md) is `Supported`. The claim is now enforced by
`scripts/check_state_ownership_coverage.py`, which walks every leaf-valued path in the persisted schema and requires
each to be claimed by exactly one entry through a coverage map **the ledger itself carries**, so the ledger stays the
authority for what it covers and the script only enforces it. Longest matching prefix wins, which is what lets a
field-level entry sit inside its container.

**The record turns on the mutations, not on the pass.** A check that has only ever passed establishes nothing, so both
failure modes were introduced into the *real* schema and observed to fire: adding a property reports an unclaimed
field, deleting a claimed one reports a rule matching nothing. Thirteen unit tests cover the seven failures the
checker distinguishes, plus the prefix rule, the container rule, and recursion termination.

**`check_v2_docs.py` runs it**, so the claim is enforced by the gate every Core V2 change already runs and by the
quality workflow behind it, rather than by remembering a script. The hook was verified the same way as the checker:
a stale rule in the ledger makes the whole documentation gate exit non-zero.

**Two independent reads found five defects in the checker and its record, and every one of them was a claim the
artifact could not support.** A map naming the same prefix twice was accepted — the exact double claim the check
exists to falsify. The scan for which entries the ledger *defines* accepted any row starting with an entry id, so
the two reference tables defined the entries they merely referenced and the two checks against an undefined entry
could never fire. That one took three passes to close: shape alone admits a ten-column row anywhere, scoping by the
ledger's table header still carried across a blank line into the next table, and a row now counts only between
that header and the first line which is not a table row. The module-parameter
subtotal was 1,116 rather than 1,128, the larger figure having swept in the position, description and scripts
leaves — and one occurrence of it survived the first correction.

**The fifth was a rule that contradicted itself, and it is the one worth keeping.** This slice first held the whole
ledger at `Investigating` until the last entry was complete, on the reasoning that a half-classified ledger reports
an unusable distinction. That is not what the register vocabulary says: `Classified` is **row-level**, so withholding
37 complete rows because 27 others are not is a rule with no basis, and it sat beside a claim that filling the
`Migration` cells alone would classify everything — which contradicted the sentence above it. The rule is now stated
row-level and, more usefully, **enforced**: an entry marked `Classified` with a blank required cell fails the gate,
which is precisely the defect that downgraded every status in this ledger once before. **37 entries are
`Classified`**; 27 are not.

**P00B-T001 still does not close, and the reason is a required column rather than the evidence.** Inventory rule 6
reads a blank field as "not yet investigated", and **27 entries have a blank `Migration` cell**. Each must state
its migration question or record an explicit `N/A` with a reason. That is the task's remaining work.

One repair beside it: `evidence/README.md` still advertised `EVD-0017` as free after that record was written.

### Completed slice — the last 27 migration cells, and what filling them found

The remaining bar was a required column, not evidence: 27 entries had a blank `Migration` cell, which inventory rule 6
reads as "not yet investigated". Each now states its migration question, or records an explicit `N/A` with a reason
where there genuinely is none — five rows take that branch: three user-settings rows with no project meaning, one
in-memory field that is never written at all, and STATE-0022's two velocity sensitivities, which *are*
project-document state but carry no identity, mirror or shape question.

Filling them was not a formality, and four rows changed what the ledger knows:

- **A module description is `graph`-dirty** (STATE-0028), which the ledger had recorded as "`graph` or `ui` — not
  established which". Settling it took two attempts, and the first was wrong in a way worth keeping: the one function
  that bumps the version has no *direct* caller, so a search for callers said no term observes the field, and the
  slice nearly shipped a second instance of STATE-0004's class on that basis. `EngineCommandSender::send` calls
  `control_snapshot::publish` before enqueueing, and its `SetModuleDescription` arm calls exactly that function. An
  independent review caught it, and a focused reread then narrowed it again: the command names an instrument, so
  what is established covers a **patch** module and not the master or return chains the same entry spans. A caller
  reached through a generic publication step is invisible to a search for direct callers, which is the method limit
  this ledger keeps rediscovering.
- **One concept has three colour encodings** in one document (STATE-0013): a hex `Option<String>` for an instrument,
  patch and module group; `TrackColor { r, g, b }` for a track, return bus and graph; and
  `SectionColor { red, green, blue }` for a section. V2 needs one, and converting the other two is the migration.
- **A user's group templates embed the project's own shapes** (STATE-0055): `Vec<ModuleState>`, `Vec<ConnectionState>`
  and `Vec<ExposedPortState>`, in files that live outside the project where no format migration reaches them. Changing
  those shapes silently breaks every template a user has saved.
- **A section holds no reference to the placements it spans** (STATE-0042); `Song` keeps the list sorted by `start` and
  nothing else, so moving a placement neither moves nor invalidates the section around it.

Method limit, recorded in the audit-pass row: every path was traced by reading rather than executing. Whether a module
description survives an actual save and reload is **not** established — the existing test exercises set, read and clear
through the bridge, not a project write — and is owed to P00B-T005.

### Selected task — P00B-T003, the identity and reference audit

Observable completion check, from the master plan's third Phase 0B exit bullet: every identity and cross-boundary
reference appears once in the [identity ledger](inventories/identities.md) with a **proposed V2 rule**. The `Proposed
V2 newtype/rule` column is blank for all 31 entries, so the task is open; its resume boundary named the two format
questions first, because [ADR-0014](decisions/ADR-0014-persistent-id-generation-and-encoding.md) is where that rule
comes from and the record says in as many words that it owes them before acceptance.

### Completed slice — the two format questions ADR-0014 owed

Both are answered, and neither changes the option ADR-0014 proposes. Their value is elsewhere: one turned into a
conversion requirement, and the other into two enforcement holes that belong to a different record.

**Can a master or return chain's module id collide with a patch's?** `IDN-0021` recorded the namespaces as different
and the overlap as unchecked. **It is real, and conditional.** Three counters allocate independently: a patch graph's
`instance_counters` keyed by module type, the master chain's `master_effect_hw`, and each return bus's
`return_effect_hw` keyed by bus *and* type. Any two owners that each hold a module of one type therefore give it the
same id — a reverb in a patch and one in the master chain are both `rev-1`. It does not follow that every project has
such a pair, and a first draft of this note claimed it did.

**It is harmless in V1 for one reason: every reference is qualified by its owner**, an `Option<InstrumentId>` or a
`ReturnBusId` on the engine commands and an `instrument` inside `AutomationTarget::Module`. The same qualification is
why **no automation lane can address a master or return-chain module at all** — there is no representable target for
one. For ADR-0014 the answer is a conversion requirement rather than a contradiction: identity is document-scoped and
every module is re-minted, so the three become three *provided* the V1 mapping is keyed by `(owner, id string)`. Keyed
by the string alone it merges them into one and silently re-points every reference at whichever survives.

**What is the closed set of parameter-name strings?** `IDN-0015` recorded it as not established. It is closed for
**73 of the 75** module types and **open for the other two**, and getting that wrong is the most useful thing this
slice did. For the 73 the set is derived rather than authored: the schema models a module as a `oneOf` per type, each
declaring its own parameter object with `additionalProperties: false`, 372 names in all, generated from the module
descriptors. For `script` and `audio_script` the names are **declared in the user's own program** — one knob per
`param`, installed into a descriptor rebuilt at load time and saved into the same parameter map.

**This slice first reported a flat closed set of 372, and an independent review refuted it.** The method is why: a
mechanical walk of the published schema cannot see a descriptor that does not exist until a script compiles. The same
correction turned up a **schema defect** — those two variants declare zero properties *and*
`additionalProperties: false`, so a project carrying any script knob is invalid against its own published schema.

Enforcement is a second question, and it belongs to ADR-0016 rather than ADR-0014. It is **not uniform**:

- **Exactly one path reports an unknown key**: the engine apply in `SynthSession::apply_patch`, on both the
  ordinary and the script-knob pass, as a `ParameterRejected` warning. **At least three skip silently** — the
  master and return chains, the GUI patch-editor restoration, and visualizer and `SignalMonitor` modules, which
  are skipped before a parameter is looked at. Two rounds of review narrowed this: the slice first said the loader
  always drops silently, then that only the two global chains do, and neither was right.
- **A lane whose `param_id` no longer resolves is a no-op** on the audio thread, which correctly cannot report it, and
  nothing reports it on the ordinary load or playback path. `rebuild_instrument_preserve_automation` **does** catch it:
  its descriptor lookup errors and the rebuild classifies any such error as an orphaned lane.

**The three rows stay `Investigating`.** Their `Known problem` cells are now answered, but the `Proposed V2
newtype/rule` column is blank, and classifying a row whose disposition is blank is exactly what downgraded the
state-ownership ledger's statuses once. That column is the rest of this task. ADR-0014 stays `Proposed`: its first
dependent implementation slice is Phase 10A, and answering what it owed is not a reason to accept it early. Its
follow-up table is updated, and its one blocked row is split: filling the ledger's **proposed-rule** column needs only
a proposed record and is `Active`, while the `Migration` column needs the conversion mapping and waits for Phase 10A.

### Remaining Phase 0B tasks

| Task           | State       | Resume boundary                                                            |
|----------------|-------------|----------------------------------------------------------------------------|
| P00B-T001      | Complete    | Closed 2026-08-29; 64 entries, all `Classified`, coverage gate-enforced   |
| P00B-T002      | Paused      | Assign reachability and migration dispositions in the capability inventory |
| P00B-T003      | Active      | Fill the `Proposed V2 newtype/rule` column for all 31 entries               |
| P00B-T004–T007, P00B-T009 | Not started | Follow the decomposition in the frozen execution record       |
| P00B-T008      | Not started | Re-scope the frozen all-ADR task under `PROCESS.md`'s decision-timing rule  |

This stream does not block Phase 3. Its detailed audit chronology remains in
the [historical Phase 0B execution record](phases/phase-00b-inventories-and-project-contracts.md); new operational state
is recorded only here.

Phase lifecycle and completed gates are recorded once in
[`ROADMAP.md`](ROADMAP.md#phase-order).

## Later owned work

- Phase 3 owns renderer ingress, the publication arbiter and producer shares,
  event scheduling, and capacity measurements. **ADR-0043's named offline
  late-clamp obligation is discharged**, by the first of its two routes: the
  selector is proved rather than rewritten, so `events_for` still windows by the
  stamp. The proof is a **tiling** property, which is what the premise reduces
  to — consecutive calls must cover contiguous quantum ranges with no gap, or an
  event falls between two windows and is skipped with nothing reporting it. The
  strain is a block size that is not a multiple of `Q`, so the carry leaves a
  different number of quanta due on successive calls and a tiling that only
  works on an aligned partition fails. Mutation-verified against a `start`
  predicate that skips the boundary quantum.
- Phase 4 owns current-project lowering and the long-running job contract.
- Phase 5 owns the `LegacyPolyModuleAdapter`'s conversion cost — the largest quantity ADR-0041 moves and the only one
  nobody has measured — and the declarative node API that `SOUND-INV-012`'s uncovered second sentence belongs to.
- Phase 9 owns completion and acceptance of ADR-0022 against retained evidence for every claimed release platform and
  initial adapter. Phase 9 may build candidates while the record is `Deferred`, but cannot exit or qualify live timing.
- Phase 0B gates Phase 10.
- ADR-0039 and `LIMIT-0017` remain Phase 10E work.
