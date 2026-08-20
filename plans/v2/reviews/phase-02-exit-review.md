# REV-P02: Phase 2 Exit Review

| Field                    | Value      |
|--------------------------|------------|
| ID                       | REV-P02    |
| Status                   | Accepted   |
| Phase                    | 02         |
| Created                  | 2026-08-20 |
| Last reviewed            | 2026-08-20 |
| Reviewed source revision | `76afd2a0` |
| Roadmap outcome          | [Phase 2 — minimal compiled voice graph](../ROADMAP.md#phase-2--minimal-compiled-voice-graph) |
| Related phase tracker    | [Phase 2 record](../phases/phase-02-minimal-compiled-voice-graph.md) (frozen) |

## Review scope

**In scope.** Everything Phase 2 added to `synth_engine_v2` at `76afd2a0`: graph validation,
compact-slot lowering, topological scheduling, the preallocated arena with liveness reuse,
the native node representation and its kernel registry, the interleaved arena conversion,
and the note edge that starts a voice. With them, the documents the phase produced or
changed: `ADR-0040`, `ADR-0041` and `ADR-0042`; `EVD-0010` through `EVD-0014`; and the
[Sound Core render contract](../specs/spec-sound-core-render-contract.md) through Phase 2.

**Out of scope, with named owners.** The phase deliberately did not build these, and each
has somewhere to land:

- renderer ingress, deferred-event storage, general event scheduling, and the ADR-0001
  clarification — Phase 3. `NoteEdge` carries no pitch and no velocity, which is scoped
  below rather than passed over silently;
- the `LegacyPolyModuleAdapter`'s conversion cost, the largest quantity ADR-0041 moves and
  the one nobody has measured — Phase 5, on ADR-0041's own follow-up table;
- the multichannel layout vocabulary (ADR-0041 clause 6) and the down-mix law — Phases 8
  and 9;
- `LIMIT-0017` and ADR-0039's public-hub contract — Phase 10E, as `NOW.md` records;
- feedback and the delay-boundary rule — ADR-0033, `Proposed`, and Phase 2 refuses every
  cycle rather than scheduling one, so nothing here assumes a cycle can be broken;
- the Phase 0B inventory and Project-contract stream, paused and not gated by this phase.

**Reviewers.** The author, plus an independent read with no memory of the intent
(`codex review --uncommitted`), as the [review protocol](../PROCESS.md#review-stopping-rule)
requires for a durable document.

## Required decisions

| ADR | Required status | Actual status | Result |
|-----|-----------------|---------------|--------|
| ADR-0004 | `Accepted` before P02-T005 | `Accepted` | Pass |
| ADR-0005 | `Accepted` before P02-T004 | `Accepted`, clauses 1, 2, 4, 5, 7 and 8 superseded by ADR-0041 | Pass — the surviving clauses 3, 6 and 9 are the ones P02-T013 was built against |
| ADR-0002 | `Accepted` before P02-T002 | `Superseded` by ADR-0041 | Pass — it was `Accepted` when P02-T002 landed, and its revisit condition fired exactly as written |
| ADR-0040 | `Accepted` | `Accepted` 2026-08-19 | Pass — accepted jointly with ADR-0041, which its clause 7 required |
| ADR-0041 | `Accepted` | `Accepted` 2026-08-19 | Pass — accepted after an independent read that refused it on four findings and a focused reread of the repair. The **five** acceptance edits its *Status* section lists have all landed, including the specification update and the index rows the documentation check compares |
| ADR-0042 | `Accepted` | `Accepted` 2026-08-20 | Pass — the disposition EVD-0013's envelope difference needed |
| ADR-0037 | `Accepted`, confirmed or superseded at this gate | `Accepted`, **confirmed** | Pass — EVD-0012, and the restriction on tuning to `Q` is discharged |
| ADR-0033 | `Proposed` permitted | `Proposed` | Pass — every cycle is refused; the cycle diagnostic promises no rule that does not exist |
| ADR-0001, ADR-0021, ADR-0032, ADR-0038 | `Accepted` | `Accepted` | Pass — carried from Phase 1 unchanged |

No decision required by this phase's entry conditions is short of the status it needs, and
no deferral is being used to carry one.

## Inventory closure

| Inventory/scope | Unclassified entries | Evidence | Result |
|-----------------|---------------------:|----------|--------|
| Resource limits, rows this phase touches | 0 | Phase 2 adds no `LIMIT` row and removes none; it consumes the `HostProfile` fields Phase 1 admitted against | Pass |
| Resource limits, whole ledger | 1 | `LIMIT-0017` is `Investigating` — 75 of 76 classified. Its disposition waits on ADR-0039, which is `Proposed` | N/A — out of scope, owned by Phase 10E |
| Reference corpus manifest | 0 | Ten cases, all with a category; the one `planned` entry is Phase 0B's sampler category and unrelated to this phase | Pass |
| Node catalog | 0 | Every registered kernel is a `node::kernels` constant, and `the_kernel_registry_is_closed_and_no_scanned_form_forges_a_kernel` checks entries and constants agree in both directions | Pass |

The eleventh corpus fixture went with P02-T006. [ADR-0040](../decisions/ADR-0040-v2-owns-its-dsp.md)
clause 5 makes no claim on it either way, and manifest coverage is P00A-T001's subject, so
its absence is not an inventory hole here.

## Exit gates

The [master plan's six Phase 2 bullets](../master-plan.md#phase-2-minimal-compiled-voice-graph), each against the check
that decides it. Two were re-decided against the **converted** renderer rather than inherited
across P02-T013, which is what the frozen record required.

| Gate | Evidence or named tests | Result |
|------|-------------------------|--------|
| The hot path contains no port strings, `HashMap` lookups, graph traversal, topology decisions, or buffer resizing | `tests/render_loop_purity.rs`, nine tests over `src/render/hot.rs` and the kernel modules: no lock, I/O or logging; no allocating construct; no panicking accessor; no topology or naming decision; every call inside the checked region; no lowercase free-function import; the region modules all scanned; and a control test asserting the scan reads the file it thinks it does. `src/tests/render_allocation.rs` arms a counting allocator after preparation and asserts zero events, including at seven irregular block sizes | **Pass**, with the stated bound below |
| The graph compiler reports a useful path-local diagnostic for an invalid cable and for a missing output path | `tests/graph_validation.rs`, 17 tests: one refusal per rule, each asserting the fields **its own** failure class requires — an unknown port named, a backwards cable called a direction mistake, a domain mismatch naming all three, competing fan-in edges named, one diagnostic per over-subscribed input, a cycle's closing edge named, a self-feeding node, and stranded sources counted | **Pass**, with the stated gap below |
| The basic voice render is musically equivalent to V1, or the difference is documented and intentional | [EVD-0013](../evidence/phase-02/EVD-0013-minimal-patch-equivalence.md). **The second branch is taken.** Every difference the record found carries a disposition traced to named code in both engines: four Explained, and two Intentional — onset placement as `CORPUS-0001-C1`, envelope segment shape as `CORPUS-0001-C2` under accepted [ADR-0042](../decisions/ADR-0042-envelope-segment-shape.md) | **Pass** — see *How this gate passes on a record that is `Not supported`* |
| Adding a second simple DSP node does not require changing renderer control flow | [ADR-0004](../decisions/ADR-0004-native-node-representation.md) clause 1 defines control flow as `render/hot.rs`, and clause 2 makes the bullet a **diff** — the amplifier's. That diff is preserved as [`ADR-0004-clause-2-amplifier-diff.patch`](../evidence/phase-02/ADR-0004-clause-2-amplifier-diff.patch), and `git apply --stat` on it reproduces clause 2's claim in one command: `ir.rs`, `node.rs`, `node/kernels.rs` and `tests/voice_nodes.rs`, and nothing else. **Re-decided at `76afd2a0`** as well: `render/hot.rs` is 583 lines and names **no concrete node kind and no specific DSP implementation**. It reaches DSP only through the generic table ADR-0004 clause 1 calls table data — `use crate::node::kernels`, `kernels::bind` at `:425`, and `step.kernel().run` at `:434` — which is the dispatch mechanism itself rather than knowledge of any kind; `IrNodeKind` does not appear in the file. A search over the crate puts every non-test node-kind reference outside the IR and registry in three `IrNodeKind::Output` `matches!` arms — `validate.rs:477`, `compile.rs:326`, `compile.rs:751` — none of which changes when a kind is added | **Pass**, on a reproducible artifact and against the converted renderer rather than inherited across the conversion |
| CPU use is no worse than V1 for the equivalent minimal patch, allowing a temporary documented margin for adapters | [EVD-0014](../evidence/phase-02/EVD-0014-minimal-patch-cpu.md), **Supported by rule 1**. V2 costs **78.0% less** than V1 on the governing pair under the conservative variant — 0.53 against 2.35 ms per second of audio — at 189 times a 0.41% noise floor, with the whole-render pair agreeing at −72.0% | **Pass**, and the temporary-margin clause is **not used**: no margin is claimed |
| The render quantum is re-measured against real V2 nodes, and ADR-0037 is confirmed or superseded | [EVD-0012](../evidence/phase-02/EVD-0012-render-quantum-real-path.md). Rule 5 fires on the voice path the phase renders, under both variants and all four estimators; a dispatch-heavy shape selected rule 2 instead, so the record escalated the disagreement rather than choosing a shape, and the user **confirmed 64** | **Pass** — ADR-0037 confirmed, and the restriction on hand-unrolled kernels, `Q`-specific layouts and control-rate-in-hertz assertions is discharged |

### How this gate passes on a record that is `Not supported`

The third bullet's evidence carries a `Not supported` conclusion, and the review states
plainly why that is not a gate failure — because reading it the other way would be the
easier and the wrong answer.

- **EVD-0013's verdict is against its own threshold E2a**, which it wrote before collection
  and which says the preferred conclusion is wrong if any declared threshold is exceeded.
  E2a is exceeded on `fall_to_50_ms`, at +20 ms against 10 ms, so the record says so.
- **The gate does not ask about E2a.** It asks for equivalence *or a documented intentional
  difference*, and ADR-0040 clause 4's *failure* branch triggers on a broken `preserve`
  claim. `CORPUS-0001-P2` claims the envelope's attack, decay, sustain and release
  **landmarks** stay within tolerance, and every landmark is within one window with the
  sustain level inside 0.0027 dB. **No `preserve` claim is broken.**
- **The excess is the release curve, and it is named in advance as intentional.** V1's
  segments are exponential and aimed past their endpoint; V2's are linear ramps of an exact
  frame count. Both arrive at the same place at the same time, which is why the landmarks
  hold, and the path between carries +1.137 dB more energy in V2 — recorded as
  `CORPUS-0001-C2` under accepted ADR-0042, measured from each engine's own gate. No fixture
  digest moved.
- **E2a was over-broad, and the record refused to fix it after the fact.** It applied a
  10 ms landmark tolerance to four fields of `EnvelopeDifference::delta_ms`, one of which is
  not a landmark on this fixture. Narrowing a threshold after seeing which of its fields
  failed is the failure mode this repository has recorded before, and EVD-0013 does not do
  it. **This review does not do it either**: the defect stays registered below, and the
  honest close is a successor record measuring P2's landmarks directly.

Five of EVD-0013's six thresholds pass, four of them by one to two orders of magnitude.

## Contract checks and specification agreement

The [render contract](../specs/spec-sound-core-render-contract.md) is `Current` and its
conformance table names a check for every invariant. Two of this phase's rows carry a stated
limit rather than a clean pass, and both are stated in the specification itself rather than
discovered here:

- **`SOUND-INV-013`** is closed in two halves and only the first by the type system. A
  `Kernel`'s field is private to `node::kernels`, so a descriptor elsewhere naming any
  function is a compile error (`E0423`, mutation-verified); every descriptor lives in
  `node.rs`, so every registered pointer is one of that module's constants. What those
  constants *wrap* is not settled by privacy — an in-module `Kernel(foreign)` is well typed
  — and a bounded source scan rejects it. Nine forging routes are mutation-checked, five of
  them found by review after a repair had looked sufficient. A scan for a grammar cannot be
  exhaustive, and the specification keeps a named entry for that narrower residual instead
  of claiming otherwise.
- **`SOUND-INV-007`'s layout half has no constructible case** while every node kind in this
  phase produces mono. `the_layout_refusal_has_no_constructible_case_in_this_phase` asserts
  that premise, so the row fails the day a stereo-output node arrives rather than going
  quiet.

### The four inherited conformance rows, audited

`NOW.md` carries this as an input to the review, and the specification records it under
*Unresolved questions*. The four rows are `SOUND-INV-001/004`, `SOUND-INV-003/012`,
`SOUND-INV-007` and `SOUND-INV-008`; two of them pair two invariants, so the audit below
takes six lines to cover four rows. **All four are index-accuracy defects: a row names a
test file that does not carry every invariant it is listed against.** None is a defect in
the code, and none was introduced by Phase 2.

| Row | What the named check actually decides | What carries the invariant |
|-----|----------------------------------------|----------------------------|
| `SOUND-INV-001` (preparation off the render path), listed against `admission`, `crate_boundary` | `admission` decides refusal before rendering; `crate_boundary` decides the crate's deletability. Neither observes the render path | `render_loop_purity`'s `the_render_loop_makes_no_topology_or_naming_decision` and `every_call_the_render_loop_makes_is_inside_the_checked_region`, plus `render_allocation` — all listed against `SOUND-INV-002` |
| `SOUND-INV-004` (admission with an attributable report) | `admission` **does** carry it | Nothing missing. The defect is the pairing: 001 and 004 share a row and only 004 is carried by it |
| `SOUND-INV-003` (immutable plan, separate per-renderer state), listed against `node_representation` | `rendering_a_plan_does_not_change_the_plan` and `one_plan_serves_two_independent_states` carry it | Nothing missing |
| `SOUND-INV-012` (closed render entry; *adding a node does not add renderer control flow*), listed against `node_representation` | The first sentence is carried. **The second sentence has no executable check at all** — it is decided by ADR-0004 clause 2's diff | Only the diff, plus the structural search this review re-ran at `76afd2a0`. This is the one row where nothing in the suite carries the claim |
| `SOUND-INV-007` (graph validation), listed against `graph_validation` | Every refusal clause is carried by a named test | The final clause — layout refusal ordered before the resource report — cannot be exercised while no node kind produces stereo, which the suite asserts rather than assumes |
| `SOUND-INV-008` (deterministic lowering, independent of declaration order), listed against `lowering` | `lowering` carries the compact-slot half | The order-independence half is decided in **`graph_validation`**: `the_schedule_is_a_function_of_identity_not_declaration_order` and `fan_out_order_is_identity_ordered_too` |

**This does not block acceptance**, and the reason is bounded rather than convenient.
Of the six invariants, four — `SOUND-INV-003`, `004`, `007` and `008` — are carried by a
named passing check, in two cases a check the row does not name; `SOUND-INV-001` is carried
by checks listed one row away, under `SOUND-INV-002`. For those five the defect is where the
index points, not whether the contract is held. **`SOUND-INV-012`'s second sentence is the
one real gap**: no check in the suite carries it. It is recorded as a residual risk below,
and it is the same claim the fourth gate bullet decides by diff — which this review
re-decided at `76afd2a0` rather than inheriting.

## Quality gates

Run at `76afd2a0` on Fedora, Linux 7.1.8, Rust stable and 1.97.0.

| Command/check | Environment | Result | Evidence |
|---------------|-------------|--------|----------|
| `python3 -B scripts/check_v2_docs.py` | Local | Pass | "Core V2 documentation checks passed." |
| `python3 -B -m unittest scripts/test_check_v2_docs.py` | Local | Pass | 15 tests, OK |
| `cargo fmt --check` | Local | Pass | Clean |
| `cargo build --workspace` | Local | Pass | No warning under `[build] warnings = "deny"` |
| `cargo clippy --workspace --all-targets` | Local | Pass | Zero warnings; Phase 2 added no new allowance |
| `cargo test --workspace` | Local | Pass | Green. `synth_engine_v2` contributes **191 passing tests across eleven binaries** plus one doc-test |
| `cargo test -p synth_engine --release resource_limit_probe_…` | Local, release | Pass | The `#[cfg(not(debug_assertions))]` branch the development run does not compile |
| `cargo doc --workspace --no-deps` | Local | Pass | No broken intra-doc link |
| `cargo check --workspace --all-targets --no-default-features` | Local | Pass | — |
| `cargo check --workspace --all-targets --all-features` | Local | Pass | — |
| `cargo +1.97.0 check --workspace` | Local, MSRV | Pass | — |
| `codex review --uncommitted` | Local | Pass after repair | Two findings, one P1 and one P2, neither in the code and both repaired; see *Outcome* |

The complete repository gate was run rather than the core one, because a phase exit should
not leave the cross-configuration and MSRV columns unread. Two `synth_engine_v2` tests are
`#[ignore]`d and that is deliberate: both are baseline **writers** —
`regenerate_baselines`, run only from the planar build, and `dump_fixture_samples`, run by
hand against a failure. Neither asserts anything, and running them in CI would overwrite the
artifact the assertion compares against.

The determinism, real-time and audio gates this phase needs are the render tests above
rather than separate commands. ADR-0041 clause 16's per-quantum digest comparison over five
fixtures is `layout_baseline`'s `every_fixture_matches_its_committed_baseline`, which is what
proved the interleaved conversion kept the audio.

## Deviations and residual risks

The frozen record's [Deviations](../phases/phase-02-minimal-compiled-voice-graph.md#deviations)
list is the register for six of these; it is not repeated. **P02-T006's dropped extraction is
registered here for the first time**, because dropping a master-plan work-list item is a
deviation and this table is the place it belongs.

| Item | Impact | Owner/task | Acceptance basis |
|------|--------|------------|------------------|
| **P02-T006's kernel extraction into `synth_dsp` does not happen** — closed, not deferred | The master plan's work list offered extraction *or* an adapter; V2 takes neither and writes its own DSP. The task carried no code, so closing it changed state and not the tree. `synth_engine_v2`'s direct dependencies are `synth_core` and `thiserror` and nothing else, asked of `cargo tree --target all` rather than of a manifest scan, so no residue remains | Closed; nothing owns follow-on work | [ADR-0040](../decisions/ADR-0040-v2-owns-its-dsp.md) clause 5, `Accepted`. The eleventh corpus fixture went with it and clause 5 makes no claim on it; manifest coverage is P00A-T001's |
| The six deviations the frozen record lists — `Gain` added ahead of P02-T005, an unreached output warning rather than a refusal, reordered admission, memory rows measured from the representation, the kernel interface made public, and `UnsupportedOutputPort` replaced | Each is recorded with its reason at the point it was taken, and none changes a gate's meaning | The frozen record | Recorded rather than absorbed, which is what the working agreement asks |
| **`SOUND-INV-012`'s second sentence has no executable check** — "adding a node does not add renderer control flow" | A regression would be invisible to a green suite. It is decided by a diff and by the structural search this review re-ran at `76afd2a0`, both of which are point-in-time | Phase 5, which builds the declarative node API this claim is really about | The search's result is unambiguous today: `render/hot.rs` names no node kind at all. A standing check would have to resolve receivers, which is the same limit the purity scan states |
| **The purity scan cannot resolve receivers** | A hot-path helper moved out of the scanned region, or a method named like an accessor on a type the file already holds, would go unseen | Any later phase touching the render path | Stated in the test rather than implied, and a control test fails if the loop leaves the file. True of any structural check; what it stops is the ordinary case |
| **`SOUND-INV-013`'s in-module residual** — a constant declared inside `node::kernels` wrapping a foreign function in a spelling the scan does not recognise | A V1 law could re-enter V2's audio path through a deliberate edit inside the one file whose purpose is to hold kernels | Named in the specification's *Unresolved questions* | Nine forging routes mutation-checked. Closing it needs a resolver-level check of the pointers the registry holds, which pointer identity across codegen units makes delicate |
| **`SOUND-INV-007`'s layout refusal has no constructible case** | The ordering clause — layout refusal before the resource report — is unexercised | The phase that adds a stereo-output node | `the_layout_refusal_has_no_constructible_case_in_this_phase` asserts the premise, so the row fails when the premise does |
| **Four inherited conformance rows name a check that does not carry the invariant** | An index defect, not a contract hole; audited row by row above | A bounded specification repair, not owned by a Phase 2 task | Every invariant except `SOUND-INV-012`'s second sentence is carried by a named passing check elsewhere in the suite |
| **`NoteEdge` carries neither pitch nor velocity** | Nothing in Phase 2 reads either, and `render.rs:171` already names where they arrive. Phase 3's ingress is the first task that must decide | Phase 3 | Deliberate in P02-T007. The decision is owed before ingress, not before this gate |
| **E2a was a stricter test than the claim it operationalised** | EVD-0013 is `Not supported` on a threshold that bounded a non-landmark field | A successor record measuring `CORPUS-0001-P2`'s landmarks directly | The defect is recorded rather than repaired, because repairing it would mean re-declaring a threshold after seeing which field failed |
| **`Q` = 64 is right for this graph, not for every graph** | A dispatch-heavy shape selects rule 2 and prefers 128; the voice path selects rule 5 and prefers 64. Every margin is 2.7 to 16.8 times its noise floor, so more measurement does not resolve it | Confirmed by the user; revisited if a later phase's catalog shifts the governing shape | EVD-0012 escalated the disagreement rather than choosing a shape. Moving 64 → 128 saves 3.6% on the voice path and costs twice the control interval and carry latency |
| **EVD-0014's ratio is an upper bound on V2's advantage, and the record overstates what that bound licenses** | V1's output module pans, limits, meters and interleaves where V2's writes to the profile's channels; that work is V1's and not V2's, so the measured 78.0% is an upper bound. EVD-0014 goes one step further — *"with that work removed V2 would win by less, and it would still win"* — and **the counterfactual was never measured**, so the sign after removing the asymmetry is not established by this record | A successor record measuring V1 with the output work removed, if a later phase needs the stronger claim | The **gate** asks whether CPU use is no worse than V1 for the equivalent minimal patch **as built**, which is what was measured; it does not ask for the counterfactual. The bullet closes on the as-built comparison, and this review does not rely on the stronger sentence |
| **EVD-0009's 865 ns against 540 ns for one fused function is an upper bound on an opportunity** | It is neither paired nor over the same computation — the fused arm skips the envelope's control work | Sizing it needs a fused arm that runs the envelope; no phase owns it | The CPU gate is measured against V1, not against a fused ideal, so this is where room exists rather than a gate failure. EVD-0014's 78% margin makes it non-urgent |
| **The `LegacyPolyModuleAdapter`'s conversion cost is unmeasured** | The largest quantity ADR-0041 moves. An interleaved V2 hosting a V1 module de-interleaves in and re-interleaves out, across the whole legacy catalog | Phase 5, on ADR-0041's follow-up table | ADR-0041 names it as unmeasured rather than assuming it small, and carries a revisit condition if it dominates |
| **`f0750c22` is cited by ADR-0004 clause 2 and the frozen phase record, and it is unreachable** | It is a pre-squash commit: the object survives in this clone but `git rev-list --all` does not list it, no branch or tag contains it, and it is not an ancestor of `HEAD`. A fresh clone cannot inspect it, and this one will lose it to `gc` | This review preserves it; the two citing documents keep the bare hash, and correcting an accepted ADR and a frozen record is not this review's to do | The diff is now committed as an evidence artifact, so the claim is checkable by `git apply --stat` without any history at all — which is stronger than the original citation was. Found by the independent read |
| **`LIMIT-0017` is `Investigating`** | One of 76 ledger rows is unclassified | Phase 10E, ADR-0039 | Out of this phase's scope; `NOW.md` records it |

## Outcome

Outcome: **Accepted.**

All six master-plan gate bullets close. Two close on executable checks that decide them; one
closes on a preserved diff plus a structural search over current source; three close on
evidence records whose falsifier and acceptance rule were written before collection. Every
decision this phase's entry conditions require holds the status it needs,
the reference corpus and the node catalog have no unclassified entry in scope, and the
complete repository gate — including the release-mode probe, both feature configurations and
the 1.97.0 MSRV check — is green at `76afd2a0`.

Two gate bullets were **re-decided against the converted renderer** rather than inherited
across P02-T013, which is what the frozen record demanded and the part of this review that
is not bookkeeping. The hot-path bullet is re-decided by a purity scan that now covers the
kernel modules; the node-addition bullet is re-decided by a structural search at `76afd2a0`
finding that `render/hot.rs` names no concrete node kind and reaches DSP only through the
generic kernel table, on top of the amplifier diff that clause 2 names — now preserved as a
committed patch rather than cited as a commit hash.

**The independent read ran twice and returned five findings. All are repaired, and none was
in the code — every one was a defect in this review.**

The first round found two. **P1**: the fourth gate bullet was accepted against `f0750c22`, a
pre-squash commit that no ref reaches and a fresh clone would not have — `git rev-list --all`
does not list it and it is not an ancestor of `HEAD`, so the evidence ADR-0004 clause 2 names
was one `gc` from gone. The diff is now committed as
[`ADR-0004-clause-2-amplifier-diff.patch`](../evidence/phase-02/ADR-0004-clause-2-amplifier-diff.patch),
and `git apply --stat` reproduces the clause's claim with no history at all. **P2**: this
review repeated EVD-0014's sentence that V2 "would still win" with V1's output work removed,
which no measurement establishes; the claim is scoped to what was measured and the overreach
is registered.

The focused reread of those repairs found three more. **The review's own structural evidence
was false**: it claimed `render/hot.rs` contains no reference to any kernel, while the file
imports `crate::node::kernels`, calls `kernels::bind` and dispatches `step.kernel().run`.
The true claim is narrower and better — the file names no *concrete* node kind and reaches
DSP only through the generic table — and an author's grep that was case-sensitive on
`Kernel` is what hid it. **The preserved patch failed `git diff --check`** on
`format-patch`'s own trailing-whitespace context and signature lines, which a scoped
`.gitattributes` exemption now covers rather than corrupting the artifact by stripping it.
And **the phase authorities had to move with this record**, which is the edit below.

This is the pattern REV-P01 recorded from the other side. There, the review found defects a
green test suite could not. Here, the code was sound and **every finding was in the document
asserting it was sound** — three of the five in claims the author had already self-audited
once.

Four properties of this phase are worth recording, because they are the transferable part.

**A measurement's verdict and its gate's verdict are different questions, and collapsing
them is the easy mistake.** EVD-0013 is `Not supported` against a threshold it wrote before
collection, and the gate it serves passes — because the gate asks for equivalence *or a
documented intentional difference*, and every difference is dispositioned. The record could
have narrowed E2a after seeing which of its four fields failed and called itself
`Supported`. It did not, and this review does not do it on the record's behalf. What that
buys is a register entry that still says a threshold was over-broad, instead of a threshold
quietly reshaped to fit.

**A control found what review did not.** V1's chain applies an equal-power centre pan three
times — the amplifier, the stereo output, and the instrument fader in `SynthEngine::process`,
the third outside the voice's module graph. Control C2 found it by failing at +3.008 dB.
Reading the chain had not found it.

**When a text scan needs a third patch, change mechanism.** `crate_boundary` was defeated by
five successive valid TOML spellings before it stopped scanning manifests and started asking
`cargo tree` — and `--target all` is required, or host-target resolution hides a
`cfg(windows)` entry. The same lesson has a second instance in this phase:
`SOUND-INV-013`'s provenance is closed by a **private field** where the type system can
carry it, and only the residue is left to a source scan, which the specification names as a
residual instead of claiming a scan for a grammar can be exhaustive.

**The largest number in the phase is a bound, not an estimate — and the record reached one
sentence past what it measured.** V2 renders the minimal patch for 78.0% less than V1, and
EVD-0014 says in the same breath that V1's output module does work V2's does not, so the
ratio is an upper bound on V2's advantage. It then adds that with that work removed V2
"would still win", and **nobody measured that counterfactual**. The independent read caught
it. The gate asks about the as-built patch and closes on the as-built comparison, so the
bullet is unaffected; the overreach is registered above rather than repeated as if
established.
