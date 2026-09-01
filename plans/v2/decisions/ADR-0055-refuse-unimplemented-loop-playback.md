# ADR-0055: Refuse Unimplemented Loop Playback

| Field | Value |
|---|---|
| ID | ADR-0055 |
| Status | Accepted |
| Phase | 3/9 |
| Created | 2026-09-01 |
| Last reviewed | 2026-09-01 |
| Related | ADR-0032, ADR-0046, ADR-0047, ADR-0050, ADR-0052, `SOUND-INV-018` |
| Supersedes | ADR-0050 clause 3's recorded-but-unenforced loop behavior and ADR-0046 clause 4's runtime-adoption sentence while this guard stands |
| Superseded by | — |

## Decision boundary

Accepting a loop changes delivered musical behavior, and adoption happens at a
real-time ownership boundary. The new refusal is also a public experimental Rust
error under the standing approval in `AGENTS.md`. A durable record is warranted
because Phase 3 exit otherwise has to choose between falsely claiming loop
playback and remaining blocked on a coupled design with no current consumer.

ADR-0052's sample-exact placement and per-pass identity remain coupled and
undecided. This record does not decide either; it decides the independent safe
boundary behavior while they remain open.

It decides only the offer-time refusal while runtime wrapping is unsupported.
It does not change loop admission, select a wrap phase, select per-pass note
identity, or provide product loop behavior. ADR-0052 remains the owner of those
coupled choices.

## Context

ADR-0050 made a loop interval part of the atomic transport-activation set while
explicitly leaving runtime wrapping unimplemented. The resulting behavior was
not fail-closed: an admitted loop activation could be offered and adopted, then
the schedule continued past the loop end as if no loop existed. The state said
that a loop was active while the audible result did not loop.

ADR-0052 shows why implementing the missing half is not a local cursor change.
Sample-exact placement and per-pass note identity must be decided together.
Keeping that coupled decision open is legitimate; silently accepting a behavior
the runtime cannot deliver is not.

## Options

1. Keep accepting loop activations and document that they do not loop. Rejected:
   successful state would continue to contradict audible behavior.
2. Reject loop requests during off-thread construction. Safe, but it prevents
   the existing density and polyphony admission checks from remaining executable
   against the eventual pass.
3. Run those checks, then refuse at the runtime offer before the exchange changes.
   Selected: the future admission contract stays tested and unsupported playback
   cannot enter active state.

## Decision

1. A transport activation carrying a loop interval may still be built off the
   audio thread. Its periodic event-density and note-polyphony checks remain
   required and run before the runtime boundary.
2. Offering that candidate to the active scheduler fails with
   `ActivationRefused::LoopPlaybackUnsupported { start, end }`. The refusal
   occurs before the exchange slot or active transport state is changed.
3. The refusal increments the stream's attributable refused-activation counter
   and returns the candidate so the control can withdraw it normally.
4. No loop interval can enter active scheduler or control state until a
   sample-exact wrap contract supersedes this guard. A non-loop activation keeps
   ADR-0050's existing offer and adoption behavior.
5. The first V2 consumer that enables loop playback must pull ADR-0052 and the
   sample-exact wrap obligation forward. Phase 9 may not exit with this refusal
   as the product's loop implementation.

## Falsifier and stopping rule

This decision is violated if a loop-bearing activation is accepted before a
sample-exact wrap exists, if refusal mutates the active schedule or loop state,
or if the diagnostic omits the rejected interval. Any of those is a
correctness defect and blocks the consuming slice. The exact wording of the
diagnostic does not.

## Evidence

- `transport_activation::runtime_loop_playback_fails_closed_until_sample_exact_wraps_exist`
  constructs a loop that passes off-thread admission and asserts the named
  interval, refused counter, unchanged scheduler/control activation sequences
  and an empty exchange after refusal.
- Active scheduler, control and retired-state types carry no loop field, so an
  accepted-but-ignored loop cannot hide in runtime state.
- The existing loop density and polyphony admission tests still exercise the
  future pass before the offer boundary.

The guard is falsified by any successful loop-bearing offer, any active loop
field reintroduced without runtime wrapping, or any refusal that changes the
schedule in force.

## Consequences and risks

- Experimental callers get an explicit unsupported-operation result instead of
  a false successful activation.
- The already-built loop admission checks remain executable and continue to
  constrain the eventual implementation.
- Loop playback is unavailable until ADR-0052 is resolved. That is deliberate
  and visible rather than an accidental partial feature.
- A caller may spend off-thread work building a loop candidate only to receive
  the explicit unsupported result at offer. This preserves admission evidence
  at the cost of later feedback.
- Revisit at the first V2 loop-playback consumer or before Phase 9 exit,
  whichever comes first.

## Specification update

`SOUND-INV-018` records the fail-closed offer boundary. ADR-0050 remains the
transport-activation contract for non-loop activations; its clause 3 loop
sentence is superseded by this record until the sample-exact successor lands.

## Review

Design consultation: a Claude Code read-only invocation on 2026-09-01 required
an executable refusal before Phase 3 could defer the sample-exact wrap. Its
falsifier was the previous behavior: a representable loop was accepted and then
silently ignored.

Independent semantic reviewer: a separate fresh Claude Code read-only
invocation reviews the final uncommitted transaction under the repository
stopping rule. Its result is recorded in REV-P03.

Stopping rule: accepted loop state without sample-exact wrapping, mutation before
refusal, or a diagnostic that cannot identify the interval blocks acceptance.
Editorial detail does not.
