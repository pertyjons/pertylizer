# DRAFT: Combined-host law for mixed producers

| Field | Value |
|---|---|
| Kind | Working draft. **Not an ADR, not registered, records no decision, lifts no refusal** |
| Author | Claude Code (Fable 5.1), 2026-10-05, at the maintainer's request |
| Inspected revision | `d07b3d83` |
| Input to | The successor record the Phase 9 exit gate allows: "ADR-0075, ADR-0048, ADR-0050 clause 8 and ADR-0051's shared-gate ownership accepted or explicitly superseded" |
| Frame it answers | [ADR-0075](ADR-0075-mixed-producer-activation-ownership.md), *Unresolved acceptance work* items 1–4 and *Split identity custody* |
| Review state | Codex read it on 2026-10-05 and found seven blocking defects (R1–R7 below). Two focused rereads accepted all seven repairs (R1–R7); the second also found a section 6/9 evidence-status contradiction, repaired by author self-audit |

## How to read this

ADR-0075 is a frame: it lists what a combined host must answer and records no answer.
[ADR-0076](ADR-0076-phase-9a-engine-gate-and-mixed-producer-deferral.md) deferred it and named
"write the combined-host law directly and test it once" as a viable option. This draft is that
law, proposed. For every open question it gives a **rule**, the **falsifier** that would show the
rule wrong, the **evidence** that would discharge it, and the **accepted clause it would amend**.

Three markers are used:

- **[V]** a code premise checked against `d07b3d83` while drafting;
- **[U]** a product choice; each is now settled under *Decisions* at the end and marked **[D]** where it is used;
- **[M]** a value or bound that only a measurement can settle.

Anything not marked [V] is design reasoning, not an observed fact. Section 9 lists what would make
the whole draft wrong.

## 1. Verified premises

Read in the code at `d07b3d83`; none was run as a new experiment.

1. **[V] An audio owner already mints compiled identities from tokens.** `CompiledLoopStream`
   stores tokenized templates (`LoopPayload::On/Off/Expression/Bend` carry a `NoteToken`) and calls
   the authoritative minter at each note-on on its render path (`src/looping/hot.rs`); it keeps a
   token-to-identity table and restores no working copy at a wrap. ADR-0065 accepted this.
2. **[V] Range-scoped minters exist and are real-time clean.** `RangeMinter`,
   `CompiledRangeMinter` and `LiveRangeMinter` (`src/identity.rs`) address only their own span;
   `mint_keyed`, `resolve` and `release` live in `src/identity/hot.rs`, the file the purity scan
   covers. Minting is a lowest-free scan over the producer's span.
3. **[V] The public live ingress mints on the offer side and redeems the hold at the offer.**
   `offer_note_on` takes the slot and hold before minting; `offer_note_off` frees the index and
   decrements `holds_outstanding` when the release takes its queue slot (`src/ingress/offers.rs`).
   The store invariant is `occupied + outstanding holds <= capacity`. `NonMonotoneStamp` is the
   fence that keeps a reused index's note-on behind the release it followed.
4. **[V] The exclusive live owner keeps key, pedal and bend state on the audio side.**
   `LiveInputStream` owns `held` cells (`id`, channel, key, `down`, optional identity) and
   per-source `pedal` and `bend` arrays (`src/host/live.rs`). An inherited bend is a second offer
   after the onset (`src/host/live/hot.rs`), which is why it can fail after onset admission.
5. **[V] A release group spends its members' reservations.** `ReleaseGroup` holds at most eight
   identities; offering one converts N reserved release slots into one queue entry
   (`src/ingress/release.rs`), charged to Live for sustain lift and panic and to Session for Stop.
6. **[V] The mixed case fails closed today.** `PerformanceIngress::prepare` returns
   `IngressPrepareError::MixedProducerPlan`; the mixed constructor and its note offers are behind
   `#[cfg(all(test, feature = "simulated-ingress"))]`.
7. **[V] The profile checks `release_event_share >= release_hold_capacity` and
   `live_event_share >= performance_ingress_capacity`, and no relation between the release share
   and the ingress depth** (`src/profile.rs`). The private mixed rehearsal store alone checks
   `release_event_share >= performance_ingress_capacity` at preparation
   (`IngressPrepareError::ReleaseBacklogShare`, test-only); the public `PerformanceIngress::prepare`
   does not. Section 6 shows why that matters.

## 2. Three structural choices

ADR-0075's open matrix is large because three things are split across threads: who mints, who may
end a live note, and how a later edge names an occurrence. Each choice below puts one of them in a
single place. They are proposals; the frame's candidate rule differs on the first.

### S1 — In a mixed owner the audio half is the only minter

**Rule.** A mixed owner's audio half holds both range minters for the owner's whole life. The
off-thread half holds no generation state. A compiled candidate carries **occupancy tokens**
instead of stamped identities: each note-on names a token, and its release, expression and bend
name the same token. Off-thread preparation runs the existing pairing book against a scratch
occupancy of the compiled span and refuses over-emission there, as stamping does today. The audio
half mints when it publishes a tokenized note-on and keeps a token table bounded by the compiled
span, not by the note count. **[D6]** A token is a **pairing key only**; it never fixes an index.
The audio half picks the index by ADR-0058 clause 2's lowest-free rule when the note-on publishes,
as the minter does today. A pre-chosen occupancy index was considered and rejected: a Panic or a
boundary release frees indices the off-thread pass still counts as held, so a later note would be
forced onto a higher index than lowest-free allows. With runtime selection, a release whose token a
mass release already ended finds a tombstone in the token table and is consumed and counted (1.6,
2.4); it never resolves through a reused index. Selection depends only on the order in which
events are applied, so it is partition-invariant. The cost is one lowest-free scan over the span
per note-on **[M]**.

**What it replaces.** The frame's candidate rule has control stamp against a copy of the compiled
range and promote it at collection in transport mode, has audio own the compiled range in loop
mode, and requires a revocation protocol between the two. Under S1 there is one custody in every
mode, so:

- ADR-0050 clause 3's working copy and its promotion at collection are unused by this owner.
  Withdrawal is exact because nothing was minted. Collection promotes only the anchor and the
  activation sequence, which are plain values.
- ADR-0050 clause 8's second obligation ("a second minting producer breaks the copy") has no copy
  to break. The two interleavings ADR-0075 requires acceptance to falsify — a live mint or release
  between candidate snapshot and promotion, and during a stamp's copy/commit window — have no
  window to occur in.
- ADR-0050 clause 5's amendment of ADR-0047 clause 7 is not needed here: the boundary release
  advances generations when it is applied, on the audio thread, which is clause 7 as written.
- Loop entry, wrap and exit need no custody transfer, because the loop owner's mechanism
  (premise 1) is already this one.

**Precedent, not novelty.** ADR-0065 moved the minter to an exclusive audio owner for the loop, and
ADR-0072 did the same for the live owner. S1 states that a mixed owner is the union of those two
accepted exceptions rather than a third custody model.

**Falsifiers.**

- An off-thread consumer needs a compiled occurrence's generation before the audio half has minted
  it. I did not search every consumer of stamped identities; this is the first thing to check.
- For one admitted event list and arm clock, the identities minted under single-callback, 64-frame,
  256-frame and irregular partitions differ.
- A callback mints, allocates, locks or finally drops outside prepared storage.
- The control half of a mixed owner exposes any mint, stamp or release API (compile-fail test).
- A withdrawn or refused candidate changes any minter, or a later build fails as over-emission
  because of it.

**Cost and limits.** The stamped-candidate, capsule-minter and promotion parts of the private
rehearsal in `src/stream/mixed.rs` would be reworked; target binding, scoped restoration, the
history and suffix builders, boundary release and arm admission stand. Erosion of the compiled
span by generation retirement becomes a terminal fault at mint, as ADR-0065 already has it, rather
than a refusal at stamping; P03-R004 owns that width. Audio-side minting cost is one span scan per
note-on **[M]**. ADR-0058 clause 6 puts a live steal's victim search "at the offer boundary", which
in this owner is the audio thread, so the first shape keeps `StealingPolicy::None`.

### S2 — Every operation that ends a live note runs on the audio half

**Rule.** Exactly five things end a live occurrence: its own key release, a pedal lift, Panic, a
whole-owner reset, and terminal teardown. All five execute on the audio half, which owns the live
range, the hold store, the registry and the occurrence table (S3). No off-thread build, offer or
collection redeems a hold.

**Consequence.** ADR-0050 clause 8's first obligation says a non-compiled producer's holds "have no
redemption authority at the boundary". Under S1 and S2 the authority and the boundary are the same
owner, so ADR-0046 clause 6's "atomically redeems every affected hold" is a local operation.

**[D1] Scope of transport Stop: compiled-scoped.** Transport commands — Start, Stop, seek, loop
entry, wrap and exit — never end, mute or re-pitch a live note. This is ADR-0050 clause 5's "a seek
moves plan time; it does not lift a performer's finger" applied to every transport command, and it
lets a performer play while the transport is stopped. A recording boundary still closes capture
at Stop; a live note held across Stop while recording is truncated in the take at Stop. Stop
while already stopped is the host's hook for the stuck-note idiom: the host may map it to Panic.
An everything-scoped Stop remains composable in the host as Stop plus Panic; the reverse is not.
This differs from V1, whose `EngineCommand::Stop` calls `all_notes_off` on every instrument.

**Falsifiers.** A compiled-scoped command changes a held live note's gate, magnitude, controller,
override, modulation layer, current ramp or next-write ramp behaviour. A hold is redeemed by
anything other than the five causes. Any redemption happens off the audio thread.

### S3 — A live occurrence is named by a handle, resolved on the audio half

**Rule.** The merger mints a `LiveOccurrenceId` (ledger cell and cell generation) when it stages an
onset. Every later command for that occurrence — release, per-note expression — names the handle.
The audio half keeps a fixed occurrence table mapping a handle to one of: `Sounding(identity)`,
`KeyUpSustained(identity)`, or a tombstone carrying the **first** terminal cause (refused with its
reason, ended by Panic, ended by reset, stolen). Channel-scoped operations (pedal, bend) carry
`(source, channel)` and the audio half resolves their members from the table, as
`LiveInputStream` does today (premise 4).

**What it answers.** ADR-0075: "The live owner's source receipt still needs a stable way to name a
later release, bend or group before an audio-owned minter returns an identity." The modeled bridge
answers it by holding a release at the merger until the onset's result returns. That couples a
short note's length to callback and collector timing. With handles the onset and its release can
travel in the same command batch.

**Falsifiers.** A release reaches a newer occurrence of the same key. A tombstone's cause is
overwritten by a later cause. A handle from a reaped cell resolves (cell generation must refuse
it). The table needs more cells than the admitted lane and hold bounds provide.

## 3. Two admission laws

Both are static, checked off the audio thread when the plan is bound, and both choose refusal over
a runtime ownership law.

### A1 — Per-note targets are partitioned; a shared playable node refuses

Adopted from the frame's target-binding prerequisite and already built in
`src/host/mixed_targets.rs`. A multi-instance node's rows belong to the producer whose identity
range contains the instance. A one-instance node reached as a playable target by both producers
refuses binding with a named diagnostic. This selects the first of ADR-0051 clause 6's three
options ("refuse target sharing across release scopes at admission") and designs no
depth-and-ownership aggregation.

**[D2] Refuse; design no aggregation now.** The refused case is unreachable from today's lowering:
the envelope is the only playable node kind (`note_control` in `src/node.rs`), the lowering places
it in voice scope, and it refuses every non-default V1 allocation mode, including mono and legato,
as Phase 6 work (`crates/pertylizer/src/lowering/render.rs`). A future mono or legato instrument
needs an aggregation law reproducing V1's single-allocator semantics, not A1 alone: with one voice
instance the two producers cannot even partition an identity range.

**Falsifier.** Binding accepts two producers whose playable targets reach one per-note row or one
one-instance node, or refuses disjoint multi-instance rows in the supported shape.

### A2 — A non-note target has one writer class

ADR-0075 requires an "explicit catch-up ownership or ordering law" for a non-note target the live
parameter lane writes. Proposed: per plan, each non-note target is written by compiled automation
**or** by the live parameter lane, never both. A live-lane target is omitted from the catch-up
batch and from adoption seeding, exactly as shared-sum and global rows are omitted in the scoped
restoration rehearsal, so a seek cannot overwrite the performer's latest value. A target with both
writers refuses binding.

**[D3] Refuse the overlap now; the later law is latch, not touch.** Each non-note target's writer
class is a static property of the binding. The host builds the live lane's slot list
(`LiveInputStream::prepare_parameters`) without automated targets and shows those controls as
locked while V2 plays. The successor law is **latch with host re-enable**, as Ableton's default
works: a live write switches the target to live-owned until a host "re-enable automation" command
switches it back at a quantum boundary, by the same transition mechanism as adoption. Touch would
also need a Session-charged return-to-automation restoration on release, so it is not the next
step.

**Influence, not only writers (R3).** Writer exclusivity does not protect live voices from a
compiled writer whose target *influences* them: a shared upstream modulator or a global parameter
feeding voice instances can be restored at a seek and change a held live note's pitch or
modulation. `MixedInstancePartition` explicitly does not classify that influence
(`src/host/mixed_targets.rs`). So A2 has a second half, which needs a new off-thread
classification of each compiled-written target by what it can reach:

- **Pre-voice influence** — a target whose value reaches any live-owned instance's local state
  through the graph before the voice sum (a global or shared upstream source, a shared-sum step
  feeding back into instances). **Refused at binding** in the first shape.
- **Post-voice timeline** — a target that acts only after the voice sum (bus, insert, mixer,
  master). Allowed. It is the timeline's own automation and is restored at a seek like any other
  compiled target; it changes the *mix* the live note is heard in, not the note's state.

S2 and rule 3.1 therefore promise preservation of a live note's **voice-local** state: gate,
magnitudes, controller and override layers, voice-local modulation, ramps and seed flags.
Post-voice automation applying to live audio is intended behaviour. **[D7]** It moves at a seek
like any other timeline automation: a held live note is heard through the destination's mix. V1's
instrument inserts are post-voice and fall on the allowed side. The risk lies entirely in the
classification, which the falsifiers below cover.

**Falsifiers.** A compiled activation changes a live-lane target's value, ramp or seed flag. A
target with both writer classes is admitted. A compiled-written target with pre-voice influence on
a live-owned instance is admitted, or a seek changes any voice-local state of a held live note. The
classification marks as post-voice a target whose value reaches an instance before the voice sum.
The catch-up batch emits more events than the compiled-owned addressable count.

## 4. The law, per open question

Numbering follows ADR-0075's *Unresolved acceptance work*.

### Item 1 — Source acceptance and release custody

The maintainer selected split outcomes on 2026-09-28; this section builds on that choice.

| # | Rule | Falsifier | Evidence |
|---|---|---|---|
| 1.1 | **Four custodies, two acceptances.** A source-ring push and a stage cell are custody. *Raw acceptance* is the raw owner's admission and decides capture only. *Sound acceptance* is HOST-INV-009's acceptance of a live note: the audio half's atomic acquisition of event slot, release hold and identity at command service. Nothing upstream predicts either | A positive outcome is reported before the consumer that owns it has decided; a mixed-lane shortage refuses or delays raw capture | `a_silent_peer_saturating_the_stage_never_refuses_raw_capture` (exists); a combined-host version over the concrete rings |
| 1.2 | **Each bounded stage admits an onset only together with its release's place.** Source ring `q + R_s <= S - 2`; raw owner `h + R <= N - 4`; stage lane one own cell plus one protected cell; command ring and result ring two cells each; ingress slot plus hold. A release for an onset a stage admitted uses that reservation and asks for nothing | A release of a sound-accepted onset is refused or dropped for capacity at any stage while each stage is saturated with ordinary packets and uncollected results | The frame's saturation test, run end to end; the per-stage halves exist in the source-ring model, the raw owner and `stage_tests` |
| 1.3 | **A stage that refuses an onset consumes its release.** One per-source/channel/key FIFO at the bridge holds accepted and refused occurrences. Source-ring packet IDs, independent of raw capture IDs, name the sound outcome; raw capture pairs its own accepted packets by the same FIFO rule while it runs. A release is delivered to each consumer that accepted the onset and consumed, with the onset's reason, for each that refused it. The bridge continues pairing after raw capture stops | A refused first onset followed by an accepted second on the same key ends the wrong note; a raw interruption changes sound pairing or loses the later release | `a_refused_then_accepted_onset_on_one_key_pairs_each_key_up_with_its_own_onset` and `a_late_raw_refusal_stops_capture_for_both_sources_while_sound_matches_ample_raw` |
| 1.4 | **Redemption boundary.** A hold is redeemed when its release edge — individual, group or mass — is written to the ingress queue or applied, on the audio thread. A key-up while the pedal is down writes no edge and redeems nothing | A hold freed at key-up while the note is sustained; a hold redeemed twice; a hold stranded after its note ended | New: sustain, lift and repeated-key cases on the mixed owner with the hold count asserted at every step |
| 1.5 | **Mass release needs no new ingress credit.** Once a pedal lift or Panic reaches the audio half, it builds groups from the occurrence table. Each member already owns a reserved slot (premise 5), so `ceil(n / 8)` group entries always fit. Groups are charged to Live; a recommended-scope Stop touches no live note. Splitting one cause into several charged groups amends ADR-0046 clause 6 and ADR-0047 clause 7 (section 7) | A pedal lift or Panic is refused or partly applied under queue pressure; a lift on one `(source, channel)` ends another's notes | New: lift and Panic with the queue full of ordinary entries and every hold outstanding |
| 1.5a | **A pedal-down reserves its lift in band; Panic has one reserved merger cell (R2).** A sustained key-up consumes its key's release reservation without redeeming the hold, so the lift needs its own. Pedal edges therefore stay **in the source FIFO**, which keeps their order against that source's key edges, and are reserved like notes: an *effective* pedal-down on a `(source, channel)` — one that changes the pedal from up to down — acquires, together with its own cell, one protected cell for its future lift at every stage upstream of ingress, exactly as an onset reserves its release (1.2). The effective lift redeems that reservation. A repeated down while down, or up while up, changes no state and is consumed at the merger and counted, so every effective lift owns a reservation. There are at most 16 such reservations per source. Nothing coalesces, so a lift at 10, key-up at 15 and down at 20 are applied in that order. **Panic** is a host command rather than a source edge: the merger owns one Panic cell, takes part in head selection with it by stamp, and refuses a second Panic at the caller boundary, before timestamped acceptance, while the cell is occupied. That is ADR-0046 clause 3's caller-boundary refusal, not a drop. At the audio half a lift or Panic builds groups as in 1.5. Raw capture keeps treating sustain as ordinary input; a raw refusal affects capture only | Saturating every stage with ordinary packets and uncollected results discards, delays without bound, or reorders an effective lift or an accepted Panic; a lift releases a note whose key-up is stamped after it, or fails to release one stamped before it; a redundant pedal edge consumes a reservation; a second Panic is lost rather than refused at the caller | New: the 1.2 saturation test extended with sustained key-ups, lift/key-up/down sequences inside one service interval, and two Panics in quick succession |
| 1.6 | **Late edges are consumed, not orphaned.** A release whose occurrence is a tombstone is consumed and counted under the tombstone's cause (`released_after_panic`, `released_after_reset`, the existing `released_after_steal`). It is not a producer defect | Such a release is counted as an orphan, or ends another note | New, one case per cause |
| 1.7 | **Inherited bend is precharged.** An onset on a channel whose bend is non-zero acquires its bend write with its slot and hold, or is refused whole. A bend change is a **coalescing marker in the live queue**: each `(source, channel)` owns one reserved queue entry, counted in the depth. The first unserviced bend places the marker at its own queue position; later bends before service update the marker's value in place. The marker applies to every sounding occurrence on that channel, with fanout bounded by them. **Ordering exception, an amendment of ADR-0023 clause 1 and SOUND-INV-020:** a bend coalesced into an earlier marker takes effect at the marker's position, at most one quantum earlier than emitted, for notes queued between the two; everything else keeps queue order | An onset is accepted and its inherited bend then fails; bend density faults the host; the last bend value is lost; an onset and a bend at one stamp apply in other than queue order, except as the stated coalescing exception allows | New. Amends ADR-0073's inherited-bend terminal policy. **[D5]**: the terminal policy is not a viable alternative — a held eight-note chord and four wheel messages in one quantum fill the default 32-entry queue and silence the host. Coalescing quantizes bend to one value per quantum per channel; raw capture keeps every message |
| 1.8 | **A terminal fault stops the whole host; a classified late raw refusal stops raw capture only.** A terminal fault silences the complete failing callback and all later output, with no rollback; joined teardown classifies every obligation without another callback. Terminal causes include an ADR-0046 clause 7 overrun, clock/pass/generation exhaustion, a plan/epoch/table mismatch, a raw admission or packet-preparation refusal while capture runs, a raw render fault, an unclassified raw self-halt, a source-ring order or serial violation, an exceeded declared source hold bound, or an expired ring-retry deadline. A classified late raw outcome after an ordered source prefix stops raw capture for both sources and retains the packet and exact reason, while both source producers and sound continue through independent source IDs. A terminal raw outcome dominates late in either completion order: it halts the whole host when discovered, without rollback, and a later late receipt cannot revive the host. Ordinary sound-stage and ingress shortages are per-occurrence refusals; violating the declared held-onset bound is terminal | An ordinary sound shortage halts the host; an unknown raw self-halt continues sound; a terminal fault leaves an obligation unclassified or resumes rendering; a classified late raw refusal stops sound or loses its exact outcome; a terminal raw outcome fails to stop sound | The concrete two-source bridge's four-partition terminal-cause matrix, single-callback unknown-raw-halt and late-raw-refusal tests, an injected direct-error latch test, and a pure terminal-priority unit test; reversed real raw completions and a real raw render fault remain untested |
| 1.9 | **Fault timing across partitions.** A fault whose cause is a function of the admitted source and command prefix occurs at the same engine quantum, with the same cause and charge report, in every partition. Delivered audio is compared only over callbacks that all succeed. A fault caused by collector or peer timing is not partition-invariant, and tests fix that schedule explicitly | A prefix-determined fault lands on different quanta under different partitions | New: one fault per closed-list cause, four partitions |
| 1.10 | **Teardown classifies state, not outcomes.** For every handle, teardown reports the actual note and hold state from the ingress queue, the faulted-callback journal and the sounding set. A release staged before a later callback failure may have spent its hold although its outcome is `Cancelled` | An accepted identity appears in zero or two locations; a still-held onset, an accepted pending release or an unexamined source packet disappears with the owner | The existing modeled teardown test; a concrete-host version |

**Liveness premise for 1.2, stated rather than assumed.** A staged packet waits until its peer's
applied frontier passes its time. The law needs a bound: each connected source publishes a
frontier at least every `F` frames **[M]**. A source whose frontier lags the audio clock by more
than a wait bound `W` **[M]** is fenced for the sound path only: the lane stops waiting for it.
Raw capture keeps its own frontier law and is unaffected.

**Execution stamp (R1).** "Executed at the fence" is not enough: once the other source has
advanced the ingress store's last accepted stamp past the fence, an offer at the fence fails
`NonMonotoneStamp` (`src/ingress/offers.rs`), and a protected release would be refused. So the
merger never offers a packet at its nominal stamp alone. It offers it at an **execution stamp**
`max(nominal, first unrendered boundary, last stamp the merger forwarded)`, which is ADR-0072's
`max(nominal, clock)` late rule plus the monotone fence the ingress already enforces. Because the
merger is the store's only offerer and computes the stamp, the store's monotone check cannot fail
for a forwarded packet; a failure is an internal defect and terminal under 1.8. The nominal stamp
stays with the packet for raw capture and for the reported outcome, which states both stamps.

To keep that delay bounded, the merger forwards a packet to ingress only once its nominal stamp
lies inside the next publication window; a future-stamped packet waits in its stage cell, not in
the ingress queue. A fenced packet is then delayed by at most `W` plus one window **[M]**.

*Falsifiers:* a silent peer delays the other source's sound beyond `W`; any forwarded packet,
including a protected release, is refused for its stamp; a fenced onset executes before an
already executed release of the index it reuses, or after a later release of its own occurrence;
an outcome reports only one of the two stamps; peer silence is reported as the waiting source's
fault.

### Item 2 — Command order and generations

**Rule.** At a quantum boundary `B` the audio half applies, in this order:

1. the fault preflight;
2. **Panic precedence (R6):** if the merger's Panic cell (1.5a) is due in this quantum, a waiting
   reset candidate is cancelled now, before any acquisition;
3. the session command queue in its own order — Stop, Start, activation, loop entry and exit.
   **Applying a Stop cancels a waiting reset candidate at that moment**, wherever the Stop stands
   in the queue, so no preflight has to find it;
4. a plan installation from the mailbox, only under the Item 3 precondition and only when neither
   step 2 nor step 3 cancelled the candidate in this quantum. Together these give ADR-0009's and
   ADR-0074's rule that an applied Stop or Panic precedes acquisition and cancels the candidate;
   the cancellation is reported as such;
5. parameter cells;
6. compiled events, with any in-quantum loop wrap as a sample-exact timeline boundary;
7. the live block: the ingress queue in queue order, including bend markers (1.7), which is where
   Panic and pedal lifts take effect.

This is ADR-0023's declared sequence with the session block made explicit, and it keeps
ADR-0046 clause 6's placement of Panic and sustain lift in the live queue. Neither step needs a
queue scan: the Panic check reads one cell, and Stop cancels as it is applied.

| # | Rule | Falsifier |
|---|---|---|
| 2.1 | **One transport change per boundary.** The single exchange slot holds one pending candidate; an offer while it is pending or retired-uncollected is refused at the caller and counted. A candidate names the sequence it supersedes; `in_force` changes at adoption alone | Two candidates adopt at one boundary; a stale candidate adopts; a refused offer changes stream state |
| 2.2 | **An adoption is three steps in fixed order:** the compiled-scoped boundary release of the old program's sounding occurrences, the anchor and program swap, then scoped restoration — all before the new program's first event | A destination-time note-on is silenced or re-pitched by restoration; an old event at or after `B` publishes; a new rising edge appears on a restored gate |
| 2.3 | **Stopped is a program state, not an owner state.** Stop suspends the compiled program after its boundary release and freezes the anchor; the owner keeps rendering live notes and release tails. A seek while stopped adopts normally and publishes no compiled event. Stop while stopped is acknowledged and changes nothing | Stop ends or mutes a live note under the recommended scope; a stopped owner publishes a compiled event; a seek while stopped is lost |
| 2.4 | **Panic is a live-queue entry with reserved storage.** It ends every sounding occurrence of both producers at its queue position. **[D4]** It leaves transport **and capture** unchanged: it ends sound, nothing else. The host's Panic button may compose Stop and Panic. This changes V2's session runtime, which today handles `SessionCommand::Panic` as a Stop with `CaptureStopReason::Panic` (`src/host/session/hot.rs`), and ADR-0072's Panic-ends-capture (section 7). Panic is soft — gates fall and tails continue under 2.5 — and V1's hard `ResetDsp` corresponds to the whole-owner reset, which 3.2 allows only while stopped. A compiled release that arrives for a token Panic ended is consumed and counted, as in 1.6 | A parameter or note flood displaces Panic; an onset queued after Panic is ended by it; a compiled release after Panic is reported as a producer defect |
| 2.5 | **A release tail is not a held note (R7).** Ending an occurrence lowers its gate and frees its identity; the voice's release stage continues to sound until that instance is reused by a later onset under ADR-0058 clause 2's lowest-free allocation, which treats a releasing voice as free, or until a reset fade. Neither the mixed owner nor a transport command shortens a tail any other way | A normal release is cut at the gate-down; an ended identity still resolves; a tail is cut by anything other than instance reuse or a reset fade |

**Evidence for Item 2.** One matrix test: Stop, Start, seek, loop entry, wrap, exit and Panic, each
at a boundary where a live note is held, a live onset and release land on the boundary, and every
credit is saturated — in both producer orders and four partitions. ADR-0075 already states that a
suite which never combines these cannot discharge the falsifiers.

### Item 3 — Owner lifetime

| # | Rule | Falsifier |
|---|---|---|
| 3.1 | **Activation and every loop transition keep the same audio owner.** The live renderer state, rows, holds and ramps survive by construction: the boundary release, restoration and adoption seeding are scoped to compiled-owned rows | A held live note is cut, retriggered, re-pitched or has its ramp jump across an activation, loop entry, wrap or exit |
| 3.2 | **A reset is a fresh owner, epoch and table, installed only while the transport is stopped** (first shape). While a compiled program runs the installation stays in its latest-wins mailbox | A reset installs under a running compiled program; a candidate from the old epoch adopts in the new owner |
| 3.3 | **A reset ends live obligations after its fade, in the old owner.** From `B` the old owner takes no input; it fades over `Q` frames; at `B + Q` its protected group ends every old note and redeems the holds; one discarded quantum lets the registry release; then it retires. Each old key-down becomes a tombstone in the new owner at `B`. This is ADR-0009 and ADR-0074 unchanged | An old identity controls a new voice; a key-up after `B` ends a note in the new owner; the old owner retires with an entry cell or hold outstanding |
| 3.4 | **No identity table is rebuilt inside a running owner.** ADR-0047 clause 8's refusal stays. ADR-0048's question is answered by 3.3: obligations stay under their old table inside the fading owner until it ends them. The successor record would supersede ADR-0048 explicitly on that basis | Any path rebuilds or replaces a table while either range has an outstanding obligation |
| 3.5 | **A reset while stopped invalidates a prepared Start.** The candidate is bound to plan, epoch, table and sequence, so the offer refuses it as stale; control withdraws it, which is free under S1, and prepares again | A Start prepared for the old plan adopts after the reset |
| 3.6 | **At most one active and one secondary owner exist.** Retired candidates, token tables, old event lists and old owners return through bounded slots and are destroyed off the audio thread | A third owner exists; a callback performs a final drop |

### Item 4 — Capacity and stealing

Design cannot settle this item; it needs the measurement ADR-0054 already requires. What the law
can fix is the set of inequalities admission must check before an owner arms, and the method.

**Admission inequalities,** per destination quantum, checked off the audio thread:

- **Session:** one boundary-release operation, plus the compiled-owned restoration groups, plus
  the parameter cells, plus one Stop, within the session share. In loop mode add ADR-0065's
  per-wrap charge times the largest wrap count one quantum can hold.
- **Compiled:** the existing sliding-window admission. A uniform shift cannot raise it, and 2.2
  suppresses the old program at `B`, so the share is not spent twice.
- **Live:** the ingress depth, which now includes one bend marker per `(source, channel)` (1.7),
  within the live share; a marker's fanout counts toward timed-control storage.
- **Release:** see section 6; the current relation is not sufficient.
- **Timed-control storage:** the compiled release queue, the restoration fanout, the suffix, every
  live release and every group member's gate and trigger writes, together.
- **[M]** payload bytes, scoped-restoration scratch, seed-scope storage and the occurrence table.

**Stealing.** `StealingPolicy::None` for both producers in the first shape. ADR-0058 is untouched
and no victim search runs on the audio thread.

**Method.** One EVD under ADR-0054, with its falsifier and acceptance rule written before
collection, on the common host: the largest survey project from EVD-0021 as the compiled load, two
live sources at saturation, an activation every few quanta, and a loop short enough to wrap more
than once per quantum. It reselects all six shares, `release_hold_capacity`,
`max_events_per_quantum` and the ingress depths together. Component evidence (EVD-0024) does not
stand in for it.

## 5. What stays refused in the first accepted shape

Each with a named diagnostic, and each a candidate for a later, separately accepted law:

- a playable one-instance node shared by both producers (A1);
- a non-note target with both writer classes (A2);
- a compiled-written target with pre-voice influence on a live-owned instance (A2, R3);
- any stealing policy other than `None`;
- a reset while a compiled program runs (3.2);
- more than one compiled and one live note producer in one plan;
- an authored runtime source in a mixed plan.

## 6. A capacity relation the current profile does not check

**Finding.** With redemption at the offer (premise 3), the number of release entries one
publication pass can charge to one destination quantum is bounded by the ingress depth, not by the
hold capacity. Let `C` be `performance_ingress_capacity`, `H` the producer's hold entitlement and
`m = min(H, C)`. Start with `m` holds outstanding and an empty queue. Offer those `m` releases;
then offer note-on/release pairs, each needing two units under `occupied + holds <= C`. The queue
then holds `m + floor((C - m) / 2)` release entries. If every stamp falls in one quantum — or every
entry is late and converges on one boundary, the case ADR-0046's audit names for the live share —
they are all charged to the release share in that quantum.

The profile requires only `release_event_share >= release_hold_capacity` (premise 7). A legal
profile with a release share equal to a small hold capacity and a deeper queue therefore lets a
conforming live producer reach `PublicationFault::ShareOverrun`, which is ADR-0046 clause 7's
terminal response. ADR-0046's audit row says the release share "covers every possible
simultaneous individual redemption"; that holds for redemption at publication, not at the offer.

**Not reachable with the engine defaults:** the default release share is 40 and the default
ingress depth is 32, so the bound is at most 32. It is a missing checked relation in the public
path, not a defect a default profile can reach. The private mixed rehearsal already refuses the
case (premise 7).

**Reproduced on 2026-10-05.** A temporary probe on the public path (the EVD-0024 capacity fixture
in `tests/simulated_ingress/capacity.rs`, not committed) used a legal harness profile with
`release_event_share = release_hold_capacity = 8`, ingress depth 32 and a plan with eight live
holds. Eight held notes were released and twelve onset/release pairs offered at one stamp: 20
releases queued, exactly `8 + floor((32 - 8) / 2)`. The next render returned
`Publication(ShareOverrun { class: Release, requested: 9, share: 8 })` with `needs_reprepare`
set, ADR-0046 clause 7's terminal response from a conforming producer.

**Scope: no stealing (R4).** The derivation holds only for `StealingPolicy::None`. Under stealing,
`publish_pending` also charges deferred releases held outside the queue: once their starts have
published, `room_for_entries` no longer counts them, `offer_note_off` spends their holds, and
`sweep` can free their identities before those releases publish (`src/ingress/offers.rs`,
`src/ingress/hot.rs`). Codex's counterexample, `C = 8` and `H = 5`, reaches nine release charges in
one quantum: five pending plus four queued. That exceeds both bounds below. A stealing store needs
its own bound that counts pending deferred releases; this draft does not derive one.

**Proposed relation, for a store without stealing.** `release_event_share >= performance_ingress_capacity` is sufficient and
simple. The tight form is `release_event_share >= m + floor((C - m) / 2)` checked at ingress
preparation, where `H` is known. This concerns the existing exclusive live owner as much as a mixed
one, so it may deserve its own slice ahead of this work.

## 7. What acceptance would amend

ADR-0075 lists the records its acceptance must amend. Under this draft:

| Record | ADR-0075 requires | Under this draft |
|---|---|---|
| ADR-0050 clause 3 | amendment | The working copy and promotion are unused by a mixed owner (S1); the exclusive path keeps them |
| ADR-0050 clause 5 | amendment | Scope stated as compiled-only for a mixed owner; its ADR-0047 clause 7 amendment does not apply there |
| ADR-0050 clause 7 | amendment | Catch-up becomes the compiled-owned scoped batch (A1, A2) |
| ADR-0050 clause 8 | amendment | Both obligations closed: S2 for holds, S1 for the second minter |
| ADR-0050 clause 9 | not listed | **New:** the audio half owns the minter in a mixed owner, as ADR-0065 and ADR-0072 already provide for theirs |
| ADR-0046 clauses 3 and 6 | amendment | Redemption at the audio-side queue entry (1.4); sound acceptance defined (1.1); one mass-release cause split into `ceil(n / 8)` charged groups (1.5) |
| ADR-0046 clause 1 | not listed | **New:** the release-share relation of section 6, for stores without stealing |
| ADR-0047 clauses 3 and 7 | amendment | Clause 3: audio-side atomic acquisition. Clause 7 (R5): **amended** — redemption at queue insertion rather than at application, and one cause charged as several groups. This extends ADR-0072's exception, which covers only its exclusive owner with at most eight voices |
| ADR-0051 clauses 4, 5, 6 | amendment | Clause 6: option one selected (A1). Clause 5: the frame's counted crossing-release exception, unchanged. Clause 4: batch size becomes the compiled-owned count |
| ADR-0072 | amendment | Its exclusive-owner mint and release exception extends to the mixed owner's audio half |
| ADR-0072 | amendment | Panic no longer ends capture or stops the transport in the mixed owner (D4) |
| ADR-0023 clause 1, SOUND-INV-020 | not listed | **New:** the bend-marker coalescing exception to queue order (1.7, D5) |
| ADR-0058 clause 2 | not listed | Unchanged: runtime lowest-free selection is kept (D6) |
| ADR-0073 | amendment | Stop scope (D1); sound-path shortages per occurrence (1.8); inherited bend (1.7, D5) |
| V2 session runtime | code change | `SessionCommand::Panic` stops ending playback and capture (D4) |
| ADR-0074 | amendment | Reset custody restricted to a stopped transport (3.2). Stop/Panic-before-acquisition is preserved by Item 2 step 2 |
| ADR-0055, ADR-0065 | amendment | Loop refusal and the no-live-ingress limit lifted for the mixed owner once its loop stage passes |
| ADR-0009 | preserve or amend | Preserved, including Stop/Panic cancelling a pending candidate (Item 2 step 2) |
| ADR-0048 | accept or supersede | Superseded by 3.4 |
| HOST-INV-009, HOST-INV-021, HOST-INV-022, SOUND-INV-017, SOUND-INV-018 | update | Updated to match the rows above. HOST-INV-021 (R5) carries both the release-share relation and the mass-release accounting that 1.5 and section 6 change |

## 8. Order of work

No production refusal is lifted until every stage has passed and the successor record with its
amendments is accepted. Each stage ends on its own falsifiers.

1. **Release-share relation** (section 6): the failing test, then the checked relation.
   Independent of everything below.
2. **Token custody** (S1) in the private owner. Exit: S1's falsifiers, with the existing
   restoration and adoption tests still passing.
3. **Occurrence table and handles** (S3) with release, sustain, pedal, Panic and bend on the mixed
   audio half. Exit: rules 1.4 to 1.7.
4. **Transport commands** on the mixed owner (S2, Item 2). Exit: the Item 2 matrix without loop.
5. **Loop mode** in the mixed owner. Exit: the matrix's loop rows; 3.1 across entry, wrap and exit.
6. **Concrete bridge:** the example's source rings to the stage and command ring under a reviewable
   API and custody contract, with the fence of Item 1. Exit: rules 1.1 to 1.3 and 1.8 to 1.10 on the
   concrete host.
7. **Reset while stopped** and joined teardown. Exit: Item 3.
8. **Capacity EVD** and ADR-0054 reselection. Exit: Item 4.

Stages 2 to 5 stay inside `synth_engine_v2` under the standing approval. Stage 6 touches the
example host. A feature edge, a shipping dependency or a production-facing API would need its own
approval.

## 9. What would make this draft wrong

- **S1 is refuted** if an off-thread consumer needs a compiled generation before audio mints it,
  or if audio-side minting cannot meet the callback budget. The fallback is the frame's split
  custody with its copy, promotion and mode-transfer protocol, and Items 1 to 4 would still apply.
- **S2's recommended scope is a product choice.** If Stop must end live notes, rule 2.3 changes and
  the mechanism does not.
- **S3 moves pairing state to the audio half.** If the occurrence table cannot be bounded by the
  admitted lane and hold counts, the modeled wait-for-result bridge is the fallback.
- **The fence in Item 1 is new.** It has no rehearsal. If executing a fenced packet late is not
  acceptable, the alternative is a per-occurrence refusal with a named reason.
- **Section 6 rests on an uncommitted probe.** Its no-stealing counterexample was reproduced once on the public path, but the probe is not in the tree, so the claim cannot be rechecked until stage 1 adds it as a permanent regression test. The stealing bound is not derived.
- **I did not read every accepted clause in full.** ADR-0050, ADR-0075 and ADR-0076 were read
  whole; ADR-0046, ADR-0051, ADR-0072 and ADR-0073 through their decision clauses; ADR-0047 in
  clauses 3, 4, 7 and 8 only; ADR-0055, ADR-0058, ADR-0065, ADR-0074, ADR-0009 and ADR-0023 in their
  decision sections; the HOST-INV and SOUND-INV texts only through the ADRs that cite them. A
  clause I did not read may contradict a rule here.
- **The draft adds no rehearsal evidence.** Every "exists" in the tables refers to a test already
  in the tree and was not re-run.

## Decisions

Taken on 2026-10-05 by Claude Code under the maintainer's explicit delegation, after an independent
consultation with a second Claude agent that had not authored this draft. The consultant agreed
on all seven, refined D3 (latch as the later law) and D4 (capture and the session runtime), and
raised D8 to D13. Its proposed occupancy-index token for D6 was rejected after Codex showed it
breaks lowest-free allocation after a Panic. The maintainer may overturn any of them; none is
accepted until the successor ADR is.

| # | Decision | Taken | Why |
|---|---|---|---|
| D1 | Stop scope | Compiled-scoped | Matches Ableton, Bitwig and hardware; the engine keeps the primitive and the host composes Stop+Panic |
| D2 | Shared playable node | Refuse at binding | Unreachable from today's lowering; mono and legato need their own law |
| D3 | Automated and live-tweaked parameter | Refuse now; later law is latch with re-enable | Latch is one writer-class transition; touch needs a restoration path |
| D4 | Panic and transport | Panic ends sound only; transport and capture continue | All-notes-off is independent of transport in mainstream hosts |
| D5 | Inherited bend | Precharge and coalesce through one queue marker per channel | The terminal policy silences the host on an ordinary wheel gesture; the marker keeps queue order except for the stated coalescing exception |
| D6 | S1 | Pursue, after stage 1; tokens are pairing keys and the audio half keeps runtime lowest-free selection | No off-thread consumer of a compiled generation was found outside the custody S1 removes. Pre-chosen indices were rejected because mass releases free indices early |
| D7 | Post-voice automation under a live note | Allowed to move at a seek | Mixer and bus automation are the timeline's in every DAW |
| D8 | Live polyphony | `StealingPolicy::None` kept for the first shape; the live producer's range is the instrument's voice count, and a refused onset is a visible counted `Dropped` | ADR-0058's victim search would run on the audio thread here; stealing for the live producer is the next law after the first shape |
| D9 | Live channel controllers (bend, sustain) reaching compiled notes | Keep them separate: live controllers reach live occurrences only | Routing them to compiled notes would make two producers write one target, which A1 and A2 refuse. Recorded as a V1 difference |
| D10 | Hard kill while playing | None in the first shape: Panic is soft, a whole-owner reset only while stopped | A reset under running playback is 3.2's open case |
| D11 | Added live latency from the fence | A fenced packet's added delay must not exceed 5 ms at any rate; the capacity EVD checks it | Below audible timing jitter for keyboard playing, with margin; `W` and the window are measured against it |
| D12 | Mono and legato in a mixed stream | Out of scope until an aggregation law exists | They fail range partition before A1 (D2) |
| D13 | Order | Stage 1 (section 6) first, then S1 (stage 2) | Stage 1 fixes the exclusive live owner that exists today |

Three of these are deliberate V1 differences — Stop scope (D1), Panic and transport (D4) and
controller scope (D9) — and must be recorded as such rather than mistaken for bugs.
