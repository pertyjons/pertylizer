# ADR-0058: Voice Allocation and Stealing

| Field | Value |
|---|---|
| ID | ADR-0058 |
| Status | Accepted |
| Phase | 6 |
| Created | 2026-09-05 |
| Last reviewed | 2026-09-05 |
| Related | ADR-0046, ADR-0047, ADR-0001, `SOUND-INV-017`, `SOUND-INV-021`, `SOUND-INV-025`, `P06-S002`, `P06-S003`, `P06-S007`, `CORPUS-0002` |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

Three boundaries. It is **delivered behaviour**: when a listener plays one note more than an
instrument has voices, which note stops and how it stops is what they hear, and a policy
changed after projects rely on it changes their sound. It is **persisted**: V1 saves
`stealing_strategy` and `max_voices` per instrument, and Phase 6's lowering must give each
saved value one meaning. And it is a **real-time and identity** boundary: a steal ends a note
that its producer has not released, so `SOUND-INV-017`'s occurrence contract must say what
that note's later release means, and the work of choosing and ending a voice must be bounded
inside the loop.

**Why it is ready now.** `P06-S001` built `N` voice instances and made every identity index
a voice (`SOUND-INV-025`); a note-on that finds no free index is refused at preparation on
the compiled path and dropped at the boundary on the live one. `P06-S002` is the next slice
and cannot proceed without this answer, and `PROCESS.md` forbids building a stealing rule
under an undecided policy. Deferring through the slice would leave the exhaustion case as
today's refusal, which `CORPUS-0002` — a polyphonic pad with voice stealing — cannot render.

**Coupled decisions.** [ADR-0047](ADR-0047-note-identity-in-the-event-contract.md) fixes that
a release names an occurrence and that an identity naming no live note is an orphan; this
record uses that vocabulary and adds one case to it. Per-note expression after a steal is
`P06-S003`'s, under the master plan's one-rule requirement, and this record states only what
the steal does to the identity that expression addresses. Mono, legato and unison
allocation — V1's `AllocationMode` and `NotePriority` — are **not** decided here: they are a
different rule (which note *sounds*, not which voice a surplus note takes) and no active
slice needs them; they are a later record when a lowering reaches them.

## Decision boundary

It decides, for a polyphonic voice scope whose producer holds every admitted index:
**which** held voice a new note-on takes, **how** the taken voice ends and when the new note
starts, what becomes of the taken note's **identity** and its pending release, **where** the
choice is made, and how V1's five saved strategies **lower**. It does not decide the fade's
shape beyond linear, mono or legato modes, sustain, or what expression a stolen voice keeps.

## Evidence

Read from V1's own allocator rather than its descriptors:

- **Five saved strategies, one default.** `voice_allocator.rs`: `None`, `Oldest` (default),
  `Quietest`, `LowestPriority`, `SameNote`; the field is `stealing_strategy` on the saved
  instrument and is set from the MCP surface and the offline settings path.
- **Three of them are one rule.** `find_voice_to_steal`: `Oldest` takes the voice with the
  greatest `age` (samples since its note-on). `Quietest` is **not** level-based — its comment
  reads "for now, use oldest releasing voice, then oldest active". `LowestPriority` orders
  releasing, then stealing, then active, and oldest within a class. Every strategy but `None`
  therefore reduces to: *a releasing voice if one exists, else the oldest*.
- **A releasing voice is already free in V2.** `SOUND-INV-017` releases the identity index at
  the note-off; the instance renders its envelope tail with no live note on it, and the next
  note-on takes the lowest free index (`IdentityTable::mint`). V1's "prefer a releasing
  voice" is therefore V2's ordinary allocation, not a steal — with one difference: V1 picks
  the *oldest* releasing voice and V2 the *lowest free index*, so which tail is cut can
  differ when two voices are releasing at once.
- **`SameNote` is a retrigger, not a steal.** `allocate_poly`: with no idle voice and
  `SameNote` set, a voice holding the same key is given the new note-on at once — no fade —
  and only if none holds it does the allocator fall back to `Oldest`.
- **A steal fades, then starts the new note late.** `Voice::steal_for` puts the voice in
  `Stealing { fade_counter: 128, pending_note }`; `instrument.rs` multiplies the voice's
  output by a linear ramp over those 128 samples (at the oversampled rate) and, when the
  counter reaches zero, resets the voice and triggers the pending note. The new note
  therefore starts **128 samples after** its event, about 2.7 ms at 48 kHz, and the old note
  is cut over that span whatever its release setting.
- **V2 today.** Over-emission on the compiled path is `IdentityError::ProducerOverEmitted`
  at stamping, which refuses the whole schedule off the audio thread; on the live path the
  note-on is dropped at the boundary with the resource named (`ExhaustedResource`). Both
  fail closed and neither steals. `EndedNote` (ADR-0050 clause 5's mass release) is the
  existing mechanism for ending a note its producer did not release.
- **The master plan's gate.** "Voice stealing begins and completes at precise sample
  offsets"; "note identity remains sufficient to route per-note expression after voice
  stealing"; `P06-S007` requires polyphonic output deterministic under stealing pressure.

## Options

Which voice:

1. **Refuse or drop, as today (`None`).** Rejected as the only policy: `CORPUS-0002` and every
   V1 project whose sequencer exceeds `max_voices` play in V1 and would not in V2. Kept as a
   declarable policy, because V1 saves it and because it is the fail-closed default that
   keeps every existing render bit-identical.
2. **The oldest held voice (`Oldest`)**, measured by note-on order. V1's default and, given
   that releasing voices are free in V2, the rule V1's `Quietest` and `LowestPriority` reduce
   to as well. Selected as the one stealing rule.
3. **Same key first, then oldest (`SameNote`).** V1's retrigger-by-key. Selected as the
   second declarable policy, with V1's shape: a held voice on the same key is retriggered at
   once, without a fade.
4. **A level-based quietest.** Rejected: V1 never implemented it, it needs envelope level
   from the audio thread at the moment of choice, and it is not deterministic across block
   sizes in the presence of smoothing.

How the taken voice ends:

5. **A hard cut at the note-on's sample.** Rejected: audible click on any sustained voice.
6. **V1's fade-then-start**: the taken voice's output fades linearly to zero over a declared
   fade length beginning at the new note's sample, and the new note starts on that instance
   when the fade completes. Selected. It is V1's audible behaviour, it is bounded work (one
   multiply per frame on one instance for the fade's span), and both edges land at exact
   sample offsets — the master plan's gate.
7. **A crossfade onto a spare instance.** Rejected for now: it needs one instance more than
   the producer's range, which breaks `SOUND-INV-025`'s identity-index-is-a-voice partition,
   and no V1 project sounds that way.

## Decision

1. A plan declares one **stealing policy** per voice scope in `PlanDeclarations`, from a
   closed set: `None`, `Oldest { fade }`, `SameNote { fade }`. The default is `None`, which is
   today's behaviour exactly, so a plan that declares nothing renders bit-identically.
2. **Exhaustion is the only trigger.** A note-on whose producer has a free index takes the
   lowest free index, as `SOUND-INV-017` already states; a releasing voice's index is free.
   Only a note-on that finds every admitted index held consults the policy.
3. **Which voice.** Under `Oldest`, the held note with the earliest note-on among the
   producer's live indices; ties cannot arise because two note-ons at one sample are ordered
   by `SOUND-INV-020`. Under `SameNote`, a held note on the same node with the same key if
   there is one — the newest such, the rule `CompiledPayload::NoteOff` already uses — else
   the oldest as under `Oldest`. Under `None`, the note-on is refused as today.
4. **How it ends.** A stolen voice's output is scaled by a linear ramp from one to zero over
   `fade` frames starting at the new note's render position, the instance's state is then
   reset to its prepared record, and the new note's gate and magnitudes land on it at
   position `+ fade` exactly — expanded and ordered as `SOUND-INV-021` states. The `fade`
   is a declared `FrameCount`; the lowering declares V1's 128. A `SameNote` retrigger on a
   held key applies the new note-on to that instance at its own position with no fade,
   which is V1's shape and what makes a repeated key sound as one repeated note.
5. **Identity.** The stolen note is **ended**, as an `EndedNote` is: its index is released
   for the new note to take, its generation advances, and its own later release names a
   superseded generation. That release is an orphan under ADR-0047 clause 4 and is refused
   as one — but it is **counted under its own name**, `released_after_steal`, not under the
   orphan count that reports a producer defect: the producer did nothing wrong. On the
   compiled path preparation knows the victim and drops the release as a named
   transformation (ADR-0001 clause 16), as it drops a crossing release today, so nothing
   reaches the audio thread to be counted. Per-note expression addressed to the stolen
   identity is likewise an orphan; what expression the *new* note inherits is `P06-S003`'s.
6. **Where the choice is made.** Off the audio thread, by the minter that already holds the
   producer's live indices and their mint order: at stamping for the compiled stream, at the
   offer boundary for the live one. The renderer receives a note-on whose expansion carries
   the steal — the victim's fade and end, and the new note's delayed start — as ordinary
   timed controls, so the loop's work is the same bounded expansion it does today plus one
   ramp. Nothing searches on the audio thread.
7. **Admission.** A plan under `Oldest` or `SameNote` charges the reset's controls to the
   same scratch a note-on's expansion is charged to. The victim's fade and the new note's
   delayed start are the note-on's expansion, and they land `fade` frames apart, so they are
   charged where they land: preparation checks the stamped positions against the compiled
   share and refuses a schedule whose steals overrun it, by name. *Corrected at build time
   from "admits a stealing note-on as one event": two events at two positions cannot be one
   charge, and admitting them as one would let a steal put more into a quantum than the
   share admits.*
8. **Lowering V1.** `None` → `None`; `Oldest`, `Quietest` and `LowestPriority` → `Oldest`;
   `SameNote` → `SameNote`; `fade` 128 in every case. The `Quietest` and `LowestPriority`
   mappings are marked `Simplified` in the lowering's fidelity, because V1 prefers the
   *oldest* releasing voice where V2 takes the *lowest free* index, so which tail is cut can
   differ when two voices release at once — the ordinary allocation, not the steal, is where
   the two diverge. `max_voices` is the producer's `simultaneous_notes`, which is already how
   `SOUND-INV-025` derives `N`.

## Falsifier and stopping rule

Violated if a note-on with a free index steals, if a steal takes any voice but the one clause
3 names, if the new note starts at any sample but position `+ fade`, if a stolen note's
release ends another note or is counted as a producer defect, if any search for a victim runs
on the audio thread, if a plan declaring `None` renders differently from today, or if two
renders of one event stream under stealing pressure differ (`P06-S007`). The fade length and
the set of declarable policies are amendable and their change is not a defect.

## Consequences and risks

- **Accepted cost.** A stealing plan carries one output ramp per instance in its scratch and
  one `FrameCount` in its declaration; the compiled stamping pass grows a victim search over
  the producer's live indices, which is bounded by `simultaneous_notes` and off-thread.
- **Risk: the late start.** V1 starts a stolen note 128 samples after its event, and this
  record keeps that rather than making the stolen note the one note V2 places later than V1.
  A project relying on the exact onset of a stolen note is a project already relying on a
  2.7 ms lag; `fade` is declared so a future lowering can shorten it deliberately.
- **Risk: `Simplified` for two V1 strategies.** The divergence is in which releasing tail is
  reused, not in stealing; it is audible only with two simultaneous release tails and a
  third note. Recorded as a fidelity mark rather than hidden.
- **Revisit condition.** A consumer that needs a crossfade onto a spare instance, a
  level-based choice, or mono/legato allocation.

## Specification update

Acceptance adds a stealing clause to `SOUND-INV-025` and the `released_after_steal` count to
`SOUND-INV-017`'s orphan cases, written by `P06-S002`, and the lowering contract gains the
clause-8 mapping when a lowering slice reaches a polyphonic instrument. No current behaviour
changes at acceptance: the default policy is `None`.

## Review

Design consultation: the options were put to the user on 2026-09-05 as two questions — which
policies, and how the taken voice ends — with the recommendation stated first in each. The
user selected `None`, `Oldest` and `SameNote` over a declared `None` default, and V1's
fade-then-start with the fade declared and 128 for a lowered V1 instrument. Accepted
2026-09-05 with that selection.

Independent semantic reviewer: the one independent read of `P06-S002`, the slice that builds
this record, reviews it with the slice; `PROCESS.md` does not require a second broad review of
the same material.

Build record: `P06-S002` built the compiled path — stamping, the activation's history and
suffix, the loop's repeating pass, the renderer's fade and reset — and corrected clause 7 as
noted there. Two facts the build fixed that the clauses leave open: a note taking a voice by
fade-then-start has its release displaced by the same `fade`, so it keeps its authored length;
and on the compiled path a release names a key, so under `SameNote` the taken note's later
release pairs with the taking note of that key (`SOUND-INV-025`'s rule) and the taking note's
own release is the one dropped — clause 5's "its own later release names a superseded
generation" reads on a release that carries an identity, which the compiled stream's does not.
`P06-S002b` built clause 6's second site. The live queue drains head-first over non-decreasing
stamps, so the deferred start waits outside it and the drain publishes it; and a hold goes with
the taken voice where every hold is outstanding, since the taken note's release is counted at
the boundary rather than queued — the hold partition of ADR-0046 clause 6 is kept, one release
still owed per reservation. Its read added one rule the clauses had left open, on both
paths: a voice committed to a deferred start or to the tail before a displaced release is not
taken, and a note-on that finds every voice so committed is an over-emission. Clause 6's second site, the live boundary, is `P06-S002b` in `NOW.md`: the
boundary holds no record of a producer's open keys, which the minter must carry before it can
choose a victim.

Stopping rule: a steal with a free index available, a victim search on the audio thread, a
stolen release ending another note, or a `None` plan rendering differently blocks acceptance.
A different fade length or an added policy does not.
