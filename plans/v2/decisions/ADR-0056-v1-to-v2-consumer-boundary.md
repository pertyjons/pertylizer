# ADR-0056: How V1 may consume V2 while V1 remains the default

| Field | Value |
|---|---|
| ID | ADR-0056 |
| Status | Accepted |
| Phase | 04 |
| Created | 2026-09-01 |
| Last reviewed | 2026-09-01 |
| Related | P04-R001, `crate_boundary`, Phase 1 exit gate, Phase 4 work list |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

The Phase 1 exit gate accepted the claim that the experimental crate "can be deleted without
affecting V1 behavior or public APIs", and `crates/synth_engine_v2/tests/crate_boundary.rs`
carries it executably: no workspace crate may reach `synth_engine_v2` through a normal or
build edge, exactly one crate may reach it through a dev edge, and exactly three measurement
harness files may name it.

Phase 4's `LegacyProjectLowerer` is the first consumer that is not a measurement harness. Its
work list places it outside the V2 core crate and lets it depend on the current project,
sequencer, module and session types, so it must reach both — which the accepted claim
forbids. The claim therefore needs a boundary rather than a deletion, and the shape of that
boundary binds every later phase: Phase 5's adapters, Phase 8's mixer and Phase 9's live
integration each add consumers, and if the boundary is not decided here each will decide it
again.

**Why now.** The active slice cannot proceed: the boundary test fails on the lowerer's first
two files, and there is no way to make it pass that does not answer this question. Deferring
would multiply reversal cost directly, since every consumer added before the answer is a
consumer to re-shape after it.

**Coupled decisions.** None. This record decides linkage and does not touch the note payload
(`P04-R001`, blocked on ADR-0025), the job contract (ADR-0028), or any persisted format.

## Decision boundary

**The choice:** by what mechanism a shipping workspace crate may link the experimental crate,
and what the boundary test asserts once it can.

**Verified premises**, measured rather than assumed:

- With default features, `cargo tree --workspace --edges normal --invert synth_engine_v2
  --target all` lists the crate alone: no dependent.
- With `--features pertylizer/v2-lowering` added to the same command, it lists exactly
  `pertylizer`.
- The existing dev-dependency is unaffected by either; the dev edge still resolves to
  `pertylizer` alone.

**Non-goals.** This record does not decide which crate hosts a lowerer, how many consumers may
exist, or when the experimental boundary ends. It decides only the linkage mechanism and what
must stay checked.

## Evidence

- `crates/synth_engine_v2/tests/crate_boundary.rs` — the five checks that carry the claim, and
  the reasoning for asking Cargo rather than scanning manifest text.
- `crates/synth_engine_v2/src/lib.rs` — the crate's own "It can be deleted" section.
- The master plan's Phase 4 work list and exit gate, whose fourth bullet requires V1 to remain
  the default for GUI, MCP, CLI and release rendering.

**Uncertainty.** The source scan that names permitted files is a scan for a literal crate
name. It does not establish reachability: a permitted file could re-export what it imports and
an unpermitted one could then use it without naming the crate. That gap is inherited, is
recorded in the existing test, and this record neither widens nor closes it.

## Options

**A. A new crate holding the lowerer.** Keeps `pertylizer` free of the edge. Rejected on a
verified fact rather than on taste: the persisted project model — `ProjectFile`,
`InstrumentState`, `Patch`, `ModuleState`, `ConnectionState` — lives in `crates/pertylizer`,
so the new crate would either depend on the application crate, whose default features link the
GUI, or duplicate the persisted model. The first inverts the dependency direction for no gain;
the second creates a second copy of a serialized contract.

**B. An unconditional dependency.** Simplest, and it deletes the property outright: a shipping
build would link the experimental crate, and the Phase 1 gate's accepted claim would become
false with nothing left checking it.

**C. A non-default feature.** The dependency is optional and reached only through a feature
that is not in `default`. The default build's dependency graph is unchanged, so the accepted
claim stays true where it is load-bearing, and the exception is one named, checkable place.

**Status quo.** Dev-dependency only. It cannot host the lowerer: a dev edge is not linked into
the library, so nothing shipping-shaped can call it, and the render and analysis harnesses the
work list targets are ordinary library code.

## Decision

**Select C.** The experimental crate is an optional dependency of `pertylizer`, enabled only
by the non-default `v2-lowering` feature.

The externally implementable consequence is what `crate_boundary` must assert from now on:

1. **With default features, no workspace crate reaches the experimental crate through a normal
   or build edge.** Unchanged in form and in strength. This is the sentence that carries the
   Phase 1 claim, and it stays exactly as strong as it was for every build that ships.
2. **With every workspace feature enabled at once, exactly one normal dependent exists, and
   it is `pertylizer`.** New, and it is what stops the feature from being a hole: without it,
   the first assertion would pass while an arbitrary number of crates linked the experimental
   crate behind features. An earlier draft asked this of the named feature alone; an
   independent review showed that naming one feature is the weakness, because a second
   optional edge behind some other crate's own feature stays switched off and therefore
   invisible. Measured: a second crate given an optional edge behind its own non-default
   feature passes both the default-features check and the named-feature check, and fails only
   this one.
3. **Enabling `v2-lowering` specifically adds exactly that one dependent.** Narrower than the
   assertion above and kept beside it, because it carries a different claim: that the
   *permitted* feature does what this record says it does.
4. **Exactly one crate reaches it through any edge kind.** Unchanged.
5. **The files that may name it are the three measurement harnesses and the lowering module
   tree, and nothing else.** Widened by exactly the module tree the feature gates.
6. **Only `v2-lowering` names the experimental crate in the feature table, and no feature
   forwards to it.** Read from the feature table rather than from the resolved graph, because
   the graph cannot tell one activating feature from two: with every feature enabled, a second
   activation produces the same single dependent and every tree assertion stays green. An
   independent review found that gap in an earlier form of this record; a second read then
   found the repair matched one activation syntax where Cargo has several, so the scan is for
   the crate's **name** rather than for `dep:`. Three routes are mutation-verified — a second
   feature naming the dependency directly, one naming `v2-lowering` at one remove, and one
   using the strong `synth_engine_v2/feature` syntax, which contains no `dep:` at all.
7. **Both manifest declarations are pinned literally**, as the dev-dependency already was, so
   that changing the form of either is a deliberate edit to this exception rather than a quiet
   reshaping.

## Consequences and risks

- **Accepted cost.** The Phase 1 gate's claim is now conditional: the crate is deletable from
  a default build, not from every build. A `--all-features` build links it, which the complete
  repository gate compiles, so the lowering path is compiled by CI even though it never ships.
  The module is gated `#[cfg(any(feature = "v2-lowering", test))]` rather than on the feature
  alone, so that the gate's `cargo test --workspace` — which resolves default features —
  actually runs the lowering tests instead of skipping them silently. That reaches the crate
  only through the dev-dependency this boundary already permits, and a `cfg(test)` build is
  not one that ships. An independent review found the tests ungated before this.
- **Safety/correctness control.** Assertion 2 is the control. The failure this boundary invites
  is a second consumer appearing behind the same or another feature, and asking Cargo with
  every feature enabled is what sees it — mutation-verified against a second crate carrying a
  hidden optional edge. Assertion 5 is the second control, at file granularity.
- **Revisit condition.** When a production dependency or a supported external consumer is
  declared, `AGENTS.md`'s standing approval for clean breaks to this crate's Rust API ends, and
  this record is revisited with it. Also revisit if a second feature ever needs the edge:
  assertions 3 and 6 name one feature, and two would need a rule rather than a name.
  Assertion 2 does not have that weakness and would keep holding.

## Specification update

No current specification changes. The contract this record creates is executable and lives in
`crates/synth_engine_v2/tests/crate_boundary.rs`; the Phase 1 exit gate's prose claim is
narrowed to the default build by the same change.

## User approval

`AGENTS.md`'s standing approval for clean breaks to this crate's Rust API expressly excludes
manifests and a shipping dependency edge, and this record creates both. The user approved
this record's selected option specifically, on 2026-09-02, after the boundary was built and
measured. The approval covers the optional feature-gated edge described here and nothing
wider: no unconditional edge, no second consumer, and no change to a persisted or wire
contract.

## Review

Reviewer: Codex, on the uncommitted change. It found that the standing approval did not cover
this edge, which is why the approval section above exists; that an earlier form of consequence
2 tested one named feature and so could not see a second optional edge behind another crate's
feature, which is why it now asks with every feature enabled; and that a return-bus subject
carried a raw `u16` where `synth_sequencer::ReturnBusId` exists. A focused reread then found
that the all-features assertion still could not distinguish one activating feature from two,
which consequence 6 closes, and that the lowerer's addresses were ordering ranks rather than
identities — an insertion sorting first repointed every address behind it. A second reread
found the widened check still blind to Cargo's `crate/feature` activation syntax, and the
lowering tests never executed by any gate command. All are repaired and mutation-verified.

Stopping rule: false conclusion-affecting fact, contradiction, unfillable contract,
safety/correctness defect, or evidence incapable of supporting the claim. Editorial detail
does not block.
