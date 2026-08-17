# REV-P01: Phase 1 Exit Review

| Field                    | Value      |
|--------------------------|------------|
| ID                       | REV-P01    |
| Status                   | Draft      |
| Phase                    | 01         |
| Created                  | 2026-08-17 |
| Last reviewed            | 2026-08-17 |
| Reviewed source revision | `ccfbc800` |
| Related phase tracker    | [Phase 1 tracker](../phases/phase-01-experimental-sound-core.md) |

## Review scope

**In scope.** The `synth_engine_v2` crate at `ccfbc800` — its ten modules, its six test
binaries, and the two manifest edits that add it to the workspace. The Phase 1 documents
changed alongside it: the tracker, the host-profile specification's six corrections, and
the master plan's `RenderConfig` sketch.

**Out of scope, with named owners.** Everything the phase deliberately did not build:

- the deferral mechanism of `HOST-INV-021`, V2's renderer-ingress streams, and the bounded
  deferred store — Phase 3, and `HOST-INV-021` is non-normative until then;
- the pre-epoch clamp and the *increment* of its counter — Phase 3's ingress mapper. The
  counter is published here, which is what ADR-0032's follow-up table assigns to Phase 1;
- graph validation, topological scheduling, the buffer arena with liveness analysis, and
  prepared/mutable state separation — Phase 2;
- the `Q` re-measurement against real V2 nodes — a Phase 2 exit-gate item under ADR-0037;
- the device path, and therefore `HOST-INV-003`'s behavioural half — Phase 9;
- `LIMIT-0004`'s job-admission error — ADR-0028, Phase 4.

**Reviewers.** The author, plus two independent passes by a reader with no memory of the
intent (`codex review`). The second pass reviewed the corrections from the first, as the
working agreement's review protocol requires.

## Required decisions

| ADR | Required status | Actual status | Result |
|-----|-----------------|---------------|--------|
| ADR-0001 | `Accepted` | `Accepted` | Pass |
| ADR-0037 | `Accepted` | `Accepted`, value provisional | Pass — nothing is tuned to 64 |
| ADR-0032 | `Accepted` | `Accepted` | Pass |
| ADR-0021 | `Accepted` | `Accepted` | Pass |
| ADR-0038 | `Accepted` | `Accepted` | Pass — its classification is consumed; `event_egress_capacity` is one field |
| ADR-0002 | `Proposed` permitted | `Proposed` | Pass — no layout vocabulary is invented |
| ADR-0009, ADR-0024, ADR-0027, ADR-0034 | `Proposed` permitted | `Proposed` | Pass — capacities carried and reported, semantics deferred |
| ADR-0022, ADR-0028 | `Deferred` permitted | `Deferred` | Pass |

## Inventory closure

| Inventory/scope | Unclassified entries | Evidence | Result |
|-----------------|---------------------:|----------|--------|
| Resource limits, `HostProfile`-owned rows | 0 | Every one of the 28 rows has a field in the profile, and `every_field_is_admitted_by_exactly_one_rule` checks the partition | Pass |
| Resource limits, everything else | — | Not this phase's; Phase 1 adds no `LIMIT` row and removes none | N/A |

## Exit gates

The master plan's [Phase 1 gate](../master-plan.md#phase-1-introduce-the-experimental-sound-core-v2-crate),
copied for review.

| Gate | Evidence or named tests | Result |
|------|-------------------------|--------|
| An empty plan and a constant/sine source render deterministically. | `an_empty_plan_renders_silence_deterministically`, `a_constant_source_renders_deterministically`, `a_sine_source_renders_deterministically_and_audibly`. Each byte-compares two renders; the sine also asserts audibility, because a determinism check alone passes on silence | Pass |
| Rendering accepts varying caller block sizes up to the configured maximum and splits every such block into the chosen fixed internal quantum. | `varying_caller_block_sizes_produce_the_same_audio` over 1, 7, 63, 64, 65, 128, 256, 1 024 and 4 096 frames against one 4 096-frame reference; `a_maximum_block_below_one_quantum_is_admitted_and_renders_the_same_audio` covers the host `HOST-INV-012` exists for | Pass |
| A plan exceeding its host profile is rejected before rendering with an attributable resource diagnostic; the renderer never silently clips the graph, event fan-out, voices, sends, or observation set to fit. | `every_limit_a_plan_can_exceed_has_a_refusal_case` proves the 28 cases *are* the admission-checked set; `each_refusal_names_its_field_both_amounts_and_the_responsible_object` runs them and asserts the plan is unchanged. Nothing clips: the only outcomes are a prepared plan and a refusal | Pass |
| The render loop takes no locks, performs no heap allocation, performs no I/O, and emits no logging. | `src/tests/render_allocation.rs` (3 tests) arms a counting allocator **before the first call after preparation** and asserts zero events, including at seven irregular block sizes and over a span that has to be sorted. `tests/render_loop_purity.rs` (4) reads `src/render/hot.rs` and bans locks, I/O, logging, panicking accessors, and allocating constructs, with a control test asserting it is reading the render loop | Pass |
| The crate can be deleted without affecting V1 behavior or public APIs. | `tests/crate_boundary.rs` (3): no workspace crate names it, its own `[dependencies]` are within a four-name allowlist, and the workspace lists it as a member — without which none of these tests would run | Pass |

## Contract checks assigned to Phase 1 by name

Not additional gates. These are the rows the accepted decisions and the `Current`
specification assign to this phase, and the gate bullets are worth nothing without them.
The [tracker](../phases/phase-01-experimental-sound-core.md) carries the table; all
fifteen rows pass, with two carrying a stated limit:

- **`HOST-INV-018`** is verified *behaviourally* — one zero-capacity case per type, and
  non-finite refused by every float constructor. The non-convertibility of `HeldNoteCount`
  and `VoiceCount` is a property of an `impl` that does not exist and is enforced by not
  writing one; no runtime test can observe it.
- **ADR-0032 clause 28's terminal fault** is implemented and exercised through the
  oversized callback, which shares the fault path exactly. Reaching clock exhaustion
  itself needs 2^64 frames — three million years at 192 kHz — so it has no test of its
  own; the arithmetic that refuses is tested directly.

## Quality gates

| Command/check | Environment | Result | Evidence |
|---------------|-------------|--------|----------|
| `cargo fmt --check` | Local, Rust 1.97 | Pass | Clean |
| `cargo build` | Local | Pass | Zero warnings under `[build] warnings = "deny"` |
| `cargo clippy --workspace --all-targets` | Local | Pass | Zero warnings |
| `cargo test --workspace` | Local | Pass | 110 tests in `synth_engine_v2` across six binaries; the rest of the workspace unchanged and green |
| `cargo doc --workspace --no-deps` | Local | Pass | No broken intra-doc link |
| `codex review --uncommitted` | Local | Pass | Two passes; eleven findings, seven P1, all fixed with a named check each |

Not run, and why: the release profile, `--no-default-features`, `--all-features`, and the
MSRV check are CI's on push, and this crate has no features and no `cfg`-gated code. The
determinism, real-time, and audio gates the phase needs are the render tests above rather
than separate commands.

## Deviations and residual risks

The tracker's [Deviations](../phases/phase-01-experimental-sound-core.md#deviations) list
is the register; nine items, each naming the document it changes. Six of them are
corrections to the host-profile specification found by writing its own conformance tests.
The residual risks:

| Item | Impact | Owner/task | Acceptance basis |
|------|--------|------------|------------------|
| `Q` = 64 is provisional | A changed constant invalidates no code here, because nothing is tuned to it | Phase 2 exit gate, ADR-0037 | The record makes the re-measurement binding, and this phase adds no dependency on the value |
| The **input carry** is prepared and never read | None today: no Phase 1 node consumes live input, and the render signature has no input block | The phase that adds an input-consuming node | ADR-0001 clause 5 sizes both carries and the report accounts for both, so the later phase does not discover it needs an allocation on the audio thread |
| Cross-platform bit-exactness of the sine is not claimed | A determinism digest compared across platforms could differ, because `f64::sin` is the platform's | Phase 3's comparison work | The gate requires deterministic rendering, which is asserted per platform; ADR-0032 clause 15's platform-independence rule binds the tempo map's conversion, not an oscillator |
| The purity check can be evaded by moving code out of `hot.rs` | A hot-path helper could be added elsewhere and go unscanned | Any later phase touching the render path | True of any structural check. What it stops is the ordinary case, and the control test fails if the loop leaves the file |
| `IrObject::Plan` attributes an aggregate count | A refusal on a count says "the plan" rather than one object | — | Correct rather than weak: an aggregate has no peak. Where one exists — memory, fan-out, script work — the specific object is named, and that is asserted |

## Outcome

Outcome: **Accepted.**

Every gate bullet has an executable check that decides it, every contract row assigned to
this phase by name passes or carries a stated and bounded limit, and the repository quality
gate is green. Eleven review findings were fixed rather than deferred, and each has a check
that would catch it again.

Two properties of the process are worth recording, because they are the transferable part.
**Writing the conformance tests is what found the specification's defects** — six of them,
including an invariant whose named test could not have passed as written, and a field in
the wrong half of the profile. Reading the specification had not found them, and neither
had four earlier review passes over it. And **the review found defects the tests did not**:
seven P1s, several of which — a control event lost between calls, a phase that grew without
bound at a negative frequency, an offline event list that silently did nothing — were
invisible to a green test suite written by the same author.
