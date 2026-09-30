# ADR-0077: Experimental V2 song playback in the application

| Field | Value |
|---|---|
| ID | ADR-0077 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-29 |
| Last reviewed | 2026-09-29 |
| Related | ADR-0022, ADR-0028, ADR-0032, ADR-0054, ADR-0057, ADR-0076, EVD-0024, P04-R004 |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

This record carries three **explicit product choices made by the user** on
2026-09-29 while running the Phase 9A engine gate
([ADR-0076](ADR-0076-phase-9a-engine-gate-and-mixed-producer-deferral.md)), and the
production-facing behaviour of the application's first V2 playback path.

**Why it is ready.** 9A gate item 1 requires V2 to be selectable for live playback. That
slice changes delivered application behaviour behind a build feature and couples to
ADR-0022, ADR-0054 and ADR-0057, so its scope must be fixed before it is built.

## Decision boundary

The user chose:

1. V2 live selection exists **only behind a non-default build feature**; V1 remains the
   default engine and the only engine of a default build.
2. In V2 mode the **MIDI keyboard is inactive**, with a visible notice explaining why. It
   is not routed to V1 in parallel.
3. Step 6 proceeds although **only four of the repository's saved projects lower** to V2
   today; every other project is refused visibly.

**Verified premises.**

- `pertylizer` already links `synth_engine_v2` only through the optional `v2-lowering`
  feature, and `crate_boundary` confines every source that names the crate to
  `src/lowering/` and the listed harnesses. The live path therefore needs **no new
  dependency edge**: it lives under `src/lowering/`, and the application refers to it only
  through `crate::lowering`.
- The lowering already builds one plan for a whole project, instruments and Phase 8 mixer
  included, but only renders it offline and discards the plan and its events.
- The V2 session transport offers Play, Stop and Panic only. It has no locate or loop
  command, and plan replacement is stopped-only.
- A midir connection is timestamp-capable; ADR-0076 decision 4 and ADR-0022 constraint 2
  keep it out of a V2 production path.
- ADR-0054's falsifier forbids starting a production live adapter under the provisional
  producer partition.

## Evidence

- `exactly_four_saved_projects_in_the_repository_lower_to_a_plan` names the four eligible
  projects; the Phase 8 corpus record names the refusals of the rest.
- [EVD-0024](../evidence/phase-09/EVD-0024-live-capacity-qualification.md) concludes that the
  provisional partition is not qualified for production live use.
- `crates/synth_engine_v2/tests/crate_boundary.rs` holds the dependency confinement.

## Options

1. **A new `v2-live` feature and module outside `src/lowering/`.** Rejected: it widens the
   boundary test and the approved edge for no behavioural gain.
2. **Full V1 transport parity in V2 mode (seek, loop, edits while playing).** Rejected for
   9A: locate, loop entry and running plan replacement on the compiled session are not
   built, and ADR-0075's mixed-producer work stays deferred.
3. **Song playback only, fail closed elsewhere.** Selected.

## Decision

1. **Gate.** The V2 playback path lives under `src/lowering/` behind `v2-lowering`. The
   application reaches it only through `crate::lowering`, under the same feature. A default
   build contains no V2 code path and no V2 setting.
2. **Scope.** V2 mode lowers the whole open project to one plan and plays it through the
   V2 session transport with its own play, pause and stop; the V1 transport does not drive
   it. Stop returns to the song start by preparing a fresh session. V2 offers no seek, loop
   or pattern preview. Entering and leaving V2 mode stops V1 and resets its DSP.
3. **Refusal.** A project with any `Refused` lowering diagnostic is not played; the notice
   names the first refusal. `Unrepresented` diagnostics play and stay visible. Project
   loading and saving are unchanged; V2 mode never writes project state.
4. **Edits.** An edit while stopped re-lowers the project off the audio thread and replaces
   the plan through the stopped-only replacement. An edit while playing is applied at the
   next pause or stop, and the notice says so. Because V2 mode cannot seek, applying it
   restarts the song from its start rather than resuming at the paused position, and a
   notice says so.
5. **Input.** MIDI input is disconnected before V2 mode is entered, with a visible notice,
   and reconnects only once the audio callback has released V2. No timestamp-capable input
   reaches V2.
6. **Capacity.** V2 mode refuses to start until an accepted evidence record has reselected
   the producer partition that song playback admits (compiled and session) under ADR-0054,
   over every saved project that lowers as a whole. The refusal is visible. *Corrected
   2026-09-29 before any measurement:* this clause first also named the release share, but
   compiled note-offs charge the Compiled class and a compiled producer holds no release
   entitlement, so song playback cannot reach the Release share or release holds; the
   evidence must show that instead of selecting them.
7. **Timing.** V2 mode claims no qualified live timing and consumes no host timestamp or
   callback latency (ADR-0022 constraint 2).

## Consequences and risks

- **Accepted cost.** V2 mode plays only the projects the lowering accepts, without seek,
  loop, live input or edits while playing. Its play and pause take effect two maximum
  callbacks plus a quantum after the request (about 170 ms at the 4096-frame ceiling),
  because a session command must land on a boundary the callback has not yet passed. Stop,
  a refused edit and leaving V2 mode silence V2 from the next callback through atomic
  flags instead. It is a development path, not a user feature.
- **Safety/correctness control.** Every unsupported case refuses visibly; the capacity gate
  holds the path closed until evidence exists; a default build is unchanged, which the
  complete repository gate's no-default-features check covers.
- **Revisit condition.** When locate, loop entry or running replacement exist for the
  compiled session, when the lowering accepts more modules, or at Phase 9's exit.

## Specification update

The [host I/O specification](../specs/spec-host-io-lifecycle.md#application-song-playback)
gains the application song-playback rules above as its current contract; conformance tests
are added there as each slice lands.

## Review

Reviewer: independent uncommitted review before commit.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract,
safety/correctness defect, or evidence incapable of supporting the claim. Editorial detail
does not block.
